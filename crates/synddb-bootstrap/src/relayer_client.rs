//! Relayer client for signature-based key registration
//!
//! The relayer handles gas payments for TEE key registration.
//! TEE signs a registration request, relayer submits to the Bridge contract.

use crate::BootstrapError;
use alloy::{
    primitives::{keccak256, Address, B256, U256},
    signers::{local::PrivateKeySigner, Signer},
};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, info};

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
    bridge_address: Address,
    chain_id: u64,
    http_client: reqwest::Client,
}

impl std::fmt::Debug for RelayerClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RelayerClient")
            .field("relayer_url", &self.relayer_url)
            .field("bridge_address", &self.bridge_address)
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
    /// Hex-encoded SP1 proof bytes
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
    pub fn new(relayer_url: String, bridge_address: Address, chain_id: u64) -> Self {
        Self {
            relayer_url,
            bridge_address,
            chain_id,
            http_client: reqwest::Client::new(),
        }
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

        // Create EIP-712 signature for registration
        let signature = self.create_registration_signature(signer, deadline).await?;

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

        let url = format!("{}/register-key", self.relayer_url);

        let response = self
            .http_client
            .post(&url)
            .json(&request)
            .timeout(Duration::from_secs(180)) // 3 minutes for registration + confirmation
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
    /// Signs the AddKey struct for the TeeKeyManager contract.
    async fn create_registration_signature(
        &self,
        signer: &PrivateKeySigner,
        deadline: u64,
    ) -> Result<Vec<u8>, BootstrapError> {
        // EIP-712 domain for TeeKeyManager
        let domain_separator = compute_domain_separator(
            "TeeKeyManager",
            "1",
            self.chain_id,
            self.bridge_address, // Bridge proxies to TeeKeyManager
        );

        // AddKey(address signer,uint256 deadline)
        let tee_key = signer.address();
        let typehash = keccak256("AddKey(address signer,uint256 deadline)");

        let struct_hash = keccak256(
            [
                typehash.as_slice(),
                &[0u8; 12], // padding for address
                tee_key.as_slice(),
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
