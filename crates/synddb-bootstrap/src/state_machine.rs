//! Bootstrap state machine

use crate::{BootstrapConfig, BootstrapError, ContractSubmitter, ProofClient, ProofResponse};
use alloy::primitives::Address;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use synddb_client::{AttestationClient, TokenType};
use synddb_shared::keys::EvmKeyManager;
use tracing::{debug, info, warn};

/// Current state of the bootstrap process
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootstrapState {
    /// Initial state, not started
    NotStarted,
    /// Key generation in progress
    GeneratingKey,
    /// Checking balance for gas
    CheckingBalance,
    /// Fetching attestation token
    FetchingAttestation,
    /// Generating SP1 proof
    GeneratingProof,
    /// Submitting to contract
    SubmittingToContract,
    /// Waiting for transaction confirmation
    AwaitingConfirmation,
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
            Self::CheckingBalance => write!(f, "checking_balance"),
            Self::FetchingAttestation => write!(f, "fetching_attestation"),
            Self::GeneratingProof => write!(f, "generating_proof"),
            Self::SubmittingToContract => write!(f, "submitting_to_contract"),
            Self::AwaitingConfirmation => write!(f, "awaiting_confirmation"),
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
}

impl std::fmt::Debug for BootstrapStateMachine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BootstrapStateMachine")
            .field("state", &self.state)
            .field("has_key", &self.key_manager.is_some())
            .field("has_attestation", &self.attestation_token.is_some())
            .field("has_proof", &self.proof.is_some())
            .field("elapsed", &self.started_at.map(|s| s.elapsed()))
            .finish()
    }
}

impl Default for BootstrapStateMachine {
    fn default() -> Self {
        Self::new()
    }
}

impl BootstrapStateMachine {
    /// Create a new bootstrap state machine
    pub const fn new() -> Self {
        Self {
            state: BootstrapState::NotStarted,
            key_manager: None,
            attestation_token: None,
            proof: None,
            started_at: None,
            last_error: None,
        }
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
        let timeout = config.bootstrap_timeout;

        // Step 1: Generate key
        self.state = BootstrapState::GeneratingKey;
        let key_manager = Arc::new(EvmKeyManager::generate());
        self.key_manager = Some(Arc::clone(&key_manager));

        let address = key_manager.address();
        let pubkey_hex = format!("0x{}", hex::encode(key_manager.public_key()));
        info!(
            address = %address,
            public_key = %pubkey_hex,
            "TEE signing key generated - fund this address before bootstrap can complete"
        );

        // Create clients
        let proof_client = ProofClient::from_config(config)?;
        let submitter = ContractSubmitter::from_config(config)?;

        // Step 2: Check/wait for balance
        self.state = BootstrapState::CheckingBalance;
        self.wait_for_balance(&submitter, address, timeout).await?;

        // Step 3: Fetch attestation token
        self.state = BootstrapState::FetchingAttestation;
        let attestation = self.fetch_attestation_with_retry(config).await?;
        self.attestation_token = Some(attestation.clone());

        // Step 4: Generate proof
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
                config.proof_max_retries,
            )
            .await?;
        self.proof = Some(proof.clone());

        // Step 5: Submit to contract
        self.state = BootstrapState::SubmittingToContract;
        let signer = key_manager.signer();
        let tx_hash = self
            .submit_with_retry(&submitter, &proof, signer, config.tx_max_retries)
            .await?;

        // Step 6: Wait for confirmation
        self.state = BootstrapState::AwaitingConfirmation;
        submitter
            .wait_for_confirmation(tx_hash, Duration::from_secs(60))
            .await?;

        // Step 7: Verify registration
        self.state = BootstrapState::VerifyingRegistration;
        let is_valid = submitter.is_key_valid(address).await?;
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
            "Key bootstrapping complete"
        );

        Ok(key_manager)
    }

    /// Wait for the address to have sufficient balance
    async fn wait_for_balance(
        &self,
        submitter: &ContractSubmitter,
        address: Address,
        timeout: Duration,
    ) -> Result<(), BootstrapError> {
        let start = Instant::now();
        let poll_interval = Duration::from_secs(5);

        loop {
            if start.elapsed() > timeout {
                return Err(BootstrapError::Timeout(timeout));
            }

            match submitter.check_balance(address).await {
                Ok(()) => {
                    info!(address = %address, "Balance check passed");
                    return Ok(());
                }
                Err(BootstrapError::InsufficientBalance { have, need }) => {
                    warn!(
                        address = %address,
                        have_wei = have,
                        need_wei = need,
                        "Waiting for funding..."
                    );
                }
                Err(e) => {
                    warn!(error = %e, "Error checking balance, retrying...");
                }
            }

            tokio::time::sleep(poll_interval).await;
        }
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
        max_retries: u32,
    ) -> Result<ProofResponse, BootstrapError> {
        let mut last_error = None;

        for attempt in 1..=max_retries {
            info!(attempt, max_retries, "Generating proof...");

            match client
                .generate_proof(attestation, audience, public_key)
                .await
            {
                Ok(proof) => return Ok(proof),
                Err(e) if e.is_retryable() => {
                    warn!(attempt, error = %e, "Proof generation failed, retrying...");
                    last_error = Some(e);

                    // Exponential backoff
                    let delay = Duration::from_secs(30 * u64::from(attempt));
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

    /// Submit transaction with retry
    async fn submit_with_retry(
        &self,
        submitter: &ContractSubmitter,
        proof: &ProofResponse,
        signer: &alloy::signers::local::PrivateKeySigner,
        max_retries: u32,
    ) -> Result<alloy::primitives::B256, BootstrapError> {
        let mut last_error = None;

        for attempt in 1..=max_retries {
            debug!(attempt, max_retries, "Submitting transaction...");

            match submitter.submit_key_registration(proof, signer).await {
                Ok(tx_hash) => return Ok(tx_hash),
                Err(e) if e.is_retryable() => {
                    warn!(attempt, error = %e, "Transaction submission failed, retrying...");
                    last_error = Some(e);

                    // Exponential backoff
                    let delay = Duration::from_secs(5 * u64::from(attempt));
                    tokio::time::sleep(delay).await;
                }
                Err(e) => return Err(e),
            }
        }

        Err(BootstrapError::MaxRetriesExceeded {
            operation: "transaction_submission".into(),
            last_error: last_error.map(|e| e.to_string()).unwrap_or_default(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_machine_new() {
        let sm = BootstrapStateMachine::new();
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
