//! Bootstrap state machine

use crate::{
    relayer_client::{KeyType, RelayerClient},
    BootstrapConfig, BootstrapError, ContractSubmitter, ProofClient, ProofResponse,
};
use alloy::primitives::Address;
use std::{sync::Arc, time::Instant};
use synddb_client::{AttestationClient, TokenType};
use synddb_shared::keys::EvmKeyManager;
use tracing::{debug, info, warn};

/// Parse a hex string (with or without 0x prefix) into bytes
fn parse_hex_bytes(hex_str: &str, field_name: &str) -> Result<Vec<u8>, BootstrapError> {
    let hex_str = hex_str.trim_start_matches("0x");
    hex::decode(hex_str)
        .map_err(|e| BootstrapError::Config(format!("Invalid {} hex encoding: {}", field_name, e)))
}

/// Current state of the bootstrap process
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootstrapState {
    /// Initial state, not started
    NotStarted,
    /// Key generation in progress
    GeneratingKey,
    /// Fetching attestation token
    FetchingAttestation,
    /// Generating SP1 proof
    GeneratingProof,
    /// Registering key via relayer
    RegisteringKey,
    /// Verifying key registration
    VerifyingRegistration,
    /// Bootstrap complete, key is registered
    Ready,
    /// Bootstrap failed
    Failed,
}

impl std::fmt::Display for BootstrapState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotStarted => write!(f, "not_started"),
            Self::GeneratingKey => write!(f, "generating_key"),
            Self::FetchingAttestation => write!(f, "fetching_attestation"),
            Self::GeneratingProof => write!(f, "generating_proof"),
            Self::RegisteringKey => write!(f, "registering_key"),
            Self::VerifyingRegistration => write!(f, "verifying_registration"),
            Self::Ready => write!(f, "ready"),
            Self::Failed => write!(f, "failed"),
        }
    }
}

/// Orchestrates the key bootstrapping process
pub struct BootstrapStateMachine {
    state: BootstrapState,
    key_manager: Option<Arc<EvmKeyManager>>,
    attestation_token: Option<String>,
    proof: Option<ProofResponse>,
    started_at: Option<Instant>,
    last_error: Option<BootstrapError>,
    key_type: KeyType,
}

impl std::fmt::Debug for BootstrapStateMachine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BootstrapStateMachine")
            .field("state", &self.state)
            .field("has_key", &self.key_manager.is_some())
            .field("has_attestation", &self.attestation_token.is_some())
            .field("has_proof", &self.proof.is_some())
            .field("elapsed", &self.started_at.map(|s| s.elapsed()))
            .field("key_type", &self.key_type)
            .finish()
    }
}

impl Default for BootstrapStateMachine {
    fn default() -> Self {
        Self::new(KeyType::Sequencer)
    }
}

impl BootstrapStateMachine {
    /// Create a new bootstrap state machine for a sequencer key
    pub const fn new(key_type: KeyType) -> Self {
        Self {
            state: BootstrapState::NotStarted,
            key_manager: None,
            attestation_token: None,
            proof: None,
            started_at: None,
            last_error: None,
            key_type,
        }
    }

    /// Create a new bootstrap state machine for a sequencer
    pub const fn for_sequencer() -> Self {
        Self::new(KeyType::Sequencer)
    }

    /// Create a new bootstrap state machine for a validator
    pub const fn for_validator() -> Self {
        Self::new(KeyType::Validator)
    }

    /// Get the current state
    pub const fn state(&self) -> BootstrapState {
        self.state
    }

    /// Get the key manager if available
    pub const fn key_manager(&self) -> Option<&Arc<EvmKeyManager>> {
        self.key_manager.as_ref()
    }

    /// Get the last error if any
    pub const fn last_error(&self) -> Option<&BootstrapError> {
        self.last_error.as_ref()
    }

    /// Get the TEE address if key has been generated
    pub fn tee_address(&self) -> Option<Address> {
        self.key_manager.as_ref().map(|k| k.address())
    }

    /// Run the bootstrap process to completion
    ///
    /// This will block until the key is registered on-chain or an error occurs.
    pub async fn run(
        &mut self,
        config: &BootstrapConfig,
    ) -> Result<Arc<EvmKeyManager>, BootstrapError> {
        // Validate config
        config.validate()?;

        self.started_at = Some(Instant::now());

        // Step 1: Generate key
        self.state = BootstrapState::GeneratingKey;
        let key_manager = Arc::new(EvmKeyManager::generate());
        self.key_manager = Some(Arc::clone(&key_manager));

        let address = key_manager.address();
        let pubkey_hex = format!("0x{}", hex::encode(key_manager.public_key()));
        info!(
            address = %address,
            public_key = %pubkey_hex,
            key_type = ?self.key_type,
            "TEE signing key generated"
        );

        // Create clients
        let proof_client = ProofClient::from_config(config)?;
        let submitter = ContractSubmitter::from_config(config)?;

        let bridge_address: Address = config
            .bridge_address
            .as_ref()
            .unwrap()
            .parse()
            .map_err(|e| BootstrapError::Config(format!("Invalid bridge address: {e}")))?;

        let rpc_url = config.rpc_url.as_ref().unwrap();

        let relayer_client = RelayerClient::new(
            config.relayer_url.clone().unwrap(),
            bridge_address,
            rpc_url,
            config.chain_id.unwrap(),
            config.relayer_timeout,
        )
        .await?;

        // Step 2: Fetch attestation token
        self.state = BootstrapState::FetchingAttestation;
        let attestation = self.fetch_attestation_with_retry(config).await?;
        self.attestation_token = Some(attestation.clone());

        // Parse image signature from config (validated in config.validate())
        // This is a 65-byte secp256k1 signature (r || s || v) for on-chain ecrecover verification
        let image_signature = parse_hex_bytes(
            config.image_signature.as_deref().unwrap(),
            "IMAGE_SIGNATURE",
        )?;
        if image_signature.len() != 65 {
            return Err(BootstrapError::Config(format!(
                "IMAGE_SIGNATURE must be exactly 65 bytes (r || s || v), got {}",
                image_signature.len()
            )));
        }

        // Step 3: Generate proof
        self.state = BootstrapState::GeneratingProof;
        let audience = config
            .attestation_audience
            .clone()
            .unwrap_or_else(|| format!("https://tee-key-manager.{}", config.chain_id.unwrap_or(1)));
        let proof = self
            .generate_proof_with_retry(
                &proof_client,
                &attestation,
                &audience,
                &key_manager.public_key(),
                &image_signature,
                config.proof_max_retries,
            )
            .await?;
        self.proof = Some(proof.clone());

        // Step 4: Register key via relayer
        self.state = BootstrapState::RegisteringKey;
        let signer = key_manager.signer();
        self.register_with_retry(&relayer_client, &proof, signer, config.relayer_max_retries)
            .await?;

        // Step 5: Verify registration
        self.state = BootstrapState::VerifyingRegistration;
        let is_valid = match self.key_type {
            KeyType::Sequencer => submitter.is_sequencer_key_valid(address).await?,
            KeyType::Validator => submitter.is_validator_key_valid(address).await?,
        };
        if !is_valid {
            return Err(BootstrapError::KeyVerificationFailed(
                "Key not found in contract after registration".into(),
            ));
        }

        self.state = BootstrapState::Ready;
        let elapsed = self.started_at.map(|s| s.elapsed()).unwrap_or_default();
        info!(
            address = %address,
            elapsed_secs = elapsed.as_secs(),
            key_type = ?self.key_type,
            "Key bootstrapping complete"
        );

        Ok(key_manager)
    }

    /// Fetch attestation token with retry
    async fn fetch_attestation_with_retry(
        &self,
        config: &BootstrapConfig,
    ) -> Result<String, BootstrapError> {
        let audience = config
            .attestation_audience
            .clone()
            .unwrap_or_else(|| format!("https://tee-key-manager.{}", config.chain_id.unwrap_or(1)));

        info!(audience = %audience, "Fetching attestation token from Confidential Space");

        // Create attestation client - this validates we're running in a TEE
        let client = AttestationClient::new(&audience, TokenType::Oidc).map_err(|e| {
            BootstrapError::AttestationFetchFailed(format!(
                "Failed to create attestation client (not running in Confidential Space?): {}",
                e
            ))
        })?;

        // Fetch the token
        let token: String = client
            .get_token()
            .await
            .map_err(|e: anyhow::Error| BootstrapError::AttestationFetchFailed(e.to_string()))?;

        info!("Successfully fetched attestation token");
        Ok(token)
    }

    /// Generate proof with retry
    async fn generate_proof_with_retry(
        &self,
        client: &ProofClient,
        attestation: &str,
        audience: &str,
        public_key: &[u8; 64],
        image_signature: &[u8],
        max_retries: u32,
    ) -> Result<ProofResponse, BootstrapError> {
        let mut last_error = None;

        for attempt in 1..=max_retries {
            info!(attempt, max_retries, "Generating proof...");

            match client
                .generate_proof(attestation, audience, public_key, image_signature)
                .await
            {
                Ok(proof) => return Ok(proof),
                Err(e) if e.is_retryable() => {
                    warn!(attempt, error = %e, "Proof generation failed, retrying...");
                    last_error = Some(e);

                    // Exponential backoff
                    let delay = std::time::Duration::from_secs(30 * u64::from(attempt));
                    tokio::time::sleep(delay).await;
                }
                Err(e) => return Err(e),
            }
        }

        Err(BootstrapError::MaxRetriesExceeded {
            operation: "proof_generation".into(),
            last_error: last_error.map(|e| e.to_string()).unwrap_or_default(),
        })
    }

    /// Register key via relayer with retry
    async fn register_with_retry(
        &self,
        relayer_client: &RelayerClient,
        proof: &ProofResponse,
        signer: &alloy::signers::local::PrivateKeySigner,
        max_retries: u32,
    ) -> Result<(), BootstrapError> {
        let mut last_error = None;

        for attempt in 1..=max_retries {
            debug!(attempt, max_retries, "Registering key via relayer...");

            match relayer_client
                .register_key(
                    signer,
                    &proof.public_values,
                    &proof.proof_bytes,
                    self.key_type,
                )
                .await
            {
                Ok(_response) => return Ok(()),
                Err(e) if e.is_retryable() => {
                    warn!(attempt, error = %e, "Key registration failed, retrying...");
                    last_error = Some(e);

                    // Exponential backoff
                    let delay = std::time::Duration::from_secs(10 * u64::from(attempt));
                    tokio::time::sleep(delay).await;
                }
                Err(e) => return Err(e),
            }
        }

        Err(BootstrapError::MaxRetriesExceeded {
            operation: "key_registration".into(),
            last_error: last_error.map(|e| e.to_string()).unwrap_or_default(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_machine_new() {
        let sm = BootstrapStateMachine::for_sequencer();
        assert_eq!(sm.state(), BootstrapState::NotStarted);
        assert!(sm.key_manager().is_none());
    }

    #[test]
    fn test_state_display() {
        assert_eq!(format!("{}", BootstrapState::NotStarted), "not_started");
        assert_eq!(
            format!("{}", BootstrapState::GeneratingProof),
            "generating_proof"
        );
        assert_eq!(format!("{}", BootstrapState::Ready), "ready");
    }
}
