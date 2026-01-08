//! Relayer client for signature-based key registration
//!
//! The relayer handles gas payments for TEE key registration.
//! TEE signs a registration request, relayer submits to the Bridge contract.

use crate::BootstrapError;
use alloy::{
    primitives::{keccak256, Address, B256, U256},
    providers::ProviderBuilder,
    signers::{local::PrivateKeySigner, Signer},
};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use synddb_bindings::bridge::Bridge::BridgeInstance;
use tracing::{debug, info};
use url::Url;

/// Timeout for GCP metadata requests
const METADATA_TIMEOUT: Duration = Duration::from_secs(5);

/// Fetch an identity token from the GCP metadata server for authenticating to Cloud Run.
/// Uses the provided HTTP client to avoid creating new connections for each request.
async fn fetch_identity_token(
    client: &reqwest::Client,
    audience: &str,
) -> Result<String, BootstrapError> {
    let metadata_url = format!(
        "http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/identity?audience={}",
        audience
    );

    let response = client
        .get(&metadata_url)
        .header("Metadata-Flavor", "Google")
        .timeout(METADATA_TIMEOUT)
        .send()
        .await
        .map_err(|e| {
            BootstrapError::ContractSubmissionFailed(format!(
                "Failed to fetch identity token: {}",
                e
            ))
        })?;

    if !response.status().is_success() {
        return Err(BootstrapError::ContractSubmissionFailed(format!(
            "Failed to fetch identity token: HTTP {}",
            response.status()
        )));
    }

    response.text().await.map_err(|e| {
        BootstrapError::ContractSubmissionFailed(format!("Failed to read identity token: {}", e))
    })
}

/// Type of key to register
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum KeyType {
    Sequencer,
    Validator,
}

/// Client for registering keys via the relayer
pub struct RelayerClient {
    relayer_url: String,
    tee_key_manager_address: Address,
    chain_id: u64,
    http_client: reqwest::Client,
}

impl std::fmt::Debug for RelayerClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RelayerClient")
            .field("relayer_url", &self.relayer_url)
            .field("tee_key_manager_address", &self.tee_key_manager_address)
            .field("chain_id", &self.chain_id)
            .finish_non_exhaustive()
    }
}

/// Request payload for key registration
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RegisterKeyRequest {
    /// Hex-encoded public values from attestation
    pub public_values: String,
    /// Hex-encoded RISC Zero proof bytes
    pub proof_bytes: String,
    /// Signature deadline (Unix timestamp)
    pub deadline: u64,
    /// Hex-encoded EIP-712 signature
    pub signature: String,
    /// Type of key (sequencer or validator)
    pub key_type: KeyType,
}

/// Response from the relayer
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterKeyResponse {
    /// Registered key address (if successful)
    pub registered_key: Option<String>,
    /// Transaction hash for key registration
    pub tx_hash: Option<String>,
    /// Error message if request failed
    pub error: Option<String>,
}

impl RelayerClient {
    /// Create a new relayer client
    ///
    /// Fetches the `TeeKeyManager` address from the Bridge contract, which is needed
    /// for EIP-712 signature verification (the domain separator uses the `TeeKeyManager` address).
    pub async fn new(
        relayer_url: String,
        bridge_address: Address,
        rpc_url: &str,
        chain_id: u64,
        timeout: Duration,
    ) -> Result<Self, BootstrapError> {
        let http_client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| BootstrapError::Config(format!("Failed to build HTTP client: {e}")))?;

        // Fetch TeeKeyManager address from Bridge contract
        let url = Url::parse(rpc_url)
            .map_err(|e| BootstrapError::Config(format!("Invalid RPC URL: {e}")))?;
        let provider = ProviderBuilder::new().connect_http(url);
        let bridge = BridgeInstance::new(bridge_address, &provider);
        let tee_key_manager_address = bridge
            .teeKeyManager()
            .call()
            .await
            .map(|r| Address::from(r.0))
            .map_err(|e| {
                BootstrapError::Config(format!("Failed to fetch TeeKeyManager address: {e}"))
            })?;

        info!(
            bridge = %bridge_address,
            tee_key_manager = %tee_key_manager_address,
            "Fetched TeeKeyManager address for EIP-712 signing"
        );

        Ok(Self {
            relayer_url,
            tee_key_manager_address,
            chain_id,
            http_client,
        })
    }

    /// Register a key via the relayer
    ///
    /// This function:
    /// 1. Creates an EIP-712 signature for the registration request
    /// 2. Sends the request to the relayer
    /// 3. Returns the registered key address
    pub async fn register_key(
        &self,
        signer: &PrivateKeySigner,
        public_values: &str,
        proof_bytes: &str,
        key_type: KeyType,
    ) -> Result<RegisterKeyResponse, BootstrapError> {
        let tee_key = signer.address();

        // Deadline: 1 hour from now
        let deadline = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 3600;

        // Parse hex values for attestation hash computation
        let public_values_bytes = hex::decode(public_values.trim_start_matches("0x"))
            .map_err(|e| BootstrapError::Config(format!("Invalid public_values hex: {e}")))?;
        let proof_bytes_bytes = hex::decode(proof_bytes.trim_start_matches("0x"))
            .map_err(|e| BootstrapError::Config(format!("Invalid proof_bytes hex: {e}")))?;

        // Create EIP-712 signature for registration
        let signature = self
            .create_registration_signature(
                signer,
                &public_values_bytes,
                &proof_bytes_bytes,
                deadline,
            )
            .await?;

        let request = RegisterKeyRequest {
            public_values: public_values.to_string(),
            proof_bytes: proof_bytes.to_string(),
            deadline,
            signature: format!("0x{}", hex::encode(&signature)),
            key_type,
        };

        info!(
            relayer = %self.relayer_url,
            tee_key = %tee_key,
            key_type = ?key_type,
            deadline = deadline,
            "Registering key via relayer"
        );

        // Fetch identity token for Cloud Run authentication (reuses client connection pool)
        let identity_token = fetch_identity_token(&self.http_client, &self.relayer_url).await?;

        let url = format!("{}/register-key", self.relayer_url);

        let response = self
            .http_client
            .post(&url)
            .header("Authorization", format!("Bearer {}", identity_token))
            .json(&request)
            .send()
            .await
            .map_err(|e| {
                BootstrapError::ContractSubmissionFailed(format!("Relayer request failed: {e}"))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(BootstrapError::ContractSubmissionFailed(format!(
                "Relayer returned {}: {}",
                status, body
            )));
        }

        let register_response: RegisterKeyResponse = response.json().await.map_err(|e| {
            BootstrapError::ContractSubmissionFailed(format!("Invalid relayer response: {e}"))
        })?;

        if let Some(error) = &register_response.error {
            return Err(BootstrapError::ContractSubmissionFailed(format!(
                "Relayer error: {}",
                error
            )));
        }

        info!(
            registered_key = ?register_response.registered_key,
            tx_hash = ?register_response.tx_hash,
            "Key registration successful"
        );

        Ok(register_response)
    }

    /// Create EIP-712 signature for key registration
    ///
    /// Signs the `AddKey` struct for the `TeeKeyManager` contract.
    /// The signature format must match the on-chain verification in `TeeKeyManager._verifyKeySignature()`.
    async fn create_registration_signature(
        &self,
        signer: &PrivateKeySigner,
        public_values: &[u8],
        proof_bytes: &[u8],
        deadline: u64,
    ) -> Result<Vec<u8>, BootstrapError> {
        // EIP-712 domain for TeeKeyManager
        // IMPORTANT: Domain uses TeeKeyManager address, not Bridge address
        let domain_separator = compute_domain_separator(
            "TeeKeyManager",
            "1",
            self.chain_id,
            self.tee_key_manager_address,
        );

        // AddKey(bytes32 attestationHash,uint256 deadline)
        // attestationHash = keccak256(abi.encodePacked(publicValues, proofBytes))
        let typehash = keccak256("AddKey(bytes32 attestationHash,uint256 deadline)");

        // Compute attestation hash (same as Solidity's abi.encodePacked)
        let attestation_hash = keccak256([public_values, proof_bytes].concat());

        // Compute struct hash using ABI encoding
        // abi.encode(typehash, attestationHash, deadline)
        let struct_hash = keccak256(
            [
                typehash.as_slice(),
                attestation_hash.as_slice(),
                &U256::from(deadline).to_be_bytes::<32>(),
            ]
            .concat(),
        );

        // Compute EIP-712 digest
        let digest = keccak256(
            [
                &[0x19, 0x01],
                domain_separator.as_slice(),
                struct_hash.as_slice(),
            ]
            .concat(),
        );

        debug!(
            domain_separator = %domain_separator,
            attestation_hash = %attestation_hash,
            struct_hash = %struct_hash,
            digest = %digest,
            "Created EIP-712 digest for registration"
        );

        // Sign the digest
        let signature = signer
            .sign_hash(&digest)
            .await
            .map_err(|e| BootstrapError::Config(format!("Signing failed: {e}")))?;

        Ok(signature.as_bytes().to_vec())
    }
}

/// Compute EIP-712 domain separator
fn compute_domain_separator(name: &str, version: &str, chain_id: u64, contract: Address) -> B256 {
    let typehash = keccak256(
        "EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)",
    );

    keccak256(
        [
            typehash.as_slice(),
            keccak256(name).as_slice(),
            keccak256(version).as_slice(),
            &U256::from(chain_id).to_be_bytes::<32>(),
            &[0u8; 12], // padding for address
            contract.as_slice(),
        ]
        .concat(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_type_serialization() {
        let json = serde_json::to_string(&KeyType::Sequencer).unwrap();
        assert_eq!(json, r#""sequencer""#);

        let json = serde_json::to_string(&KeyType::Validator).unwrap();
        assert_eq!(json, r#""validator""#);
    }

    #[test]
    fn test_domain_separator_computation() {
        let separator = compute_domain_separator("TeeKeyManager", "1", 1, Address::ZERO);
        // Just verify it produces a valid hash
        assert_ne!(separator, B256::ZERO);
    }
}
