//! Client for the GPU proof generation service

use crate::{BootstrapConfig, BootstrapError, ProverMode};
use alloy::primitives::Address;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, info, warn};

/// Request to generate an attestation proof
#[derive(Debug, Clone, Serialize)]
struct ProofRequest {
    /// Raw JWT attestation token from Confidential Space
    pub jwt_token: String,
    /// Expected audience claim
    pub expected_audience: String,
    /// EVM public key (64-byte uncompressed secp256k1, hex-encoded)
    pub evm_public_key: String,
    /// Cosign signature over image digest (64 bytes: r || s, hex-encoded)
    pub cosign_signature: String,
    /// Cosign public key (64 or 65 bytes, hex-encoded)
    pub cosign_pubkey: String,
}

/// Response from the proof service
#[derive(Debug, Clone, Deserialize)]
pub struct ProofResponse {
    /// ABI-encoded `PublicValuesStruct` (hex)
    pub public_values: String,
    /// SP1 proof bytes (hex)
    pub proof_bytes: String,
    /// Derived TEE address for verification
    pub tee_address: String,
}

/// Client for communicating with the proof generation service
#[derive(Debug)]
pub struct ProofClient {
    client: reqwest::Client,
    service_url: String,
    timeout: Duration,
    health_check_timeout: Duration,
}

impl ProofClient {
    /// Create a new proof client from config
    pub fn from_config(config: &BootstrapConfig) -> Result<Self, BootstrapError> {
        let service_url = match config.prover_mode {
            ProverMode::Service => config
                .proof_service_url
                .clone()
                .ok_or_else(|| BootstrapError::Config("PROOF_SERVICE_URL is required".into()))?,
            ProverMode::Mock => "mock://localhost".into(),
        };

        let client = reqwest::Client::builder()
            .timeout(config.proof_timeout)
            .build()
            .map_err(|e| BootstrapError::ProofServiceUnavailable(e.to_string()))?;

        Ok(Self {
            client,
            service_url,
            timeout: config.proof_timeout,
            health_check_timeout: config.proof_health_check_timeout,
        })
    }

    /// Generate a proof for the given attestation
    ///
    /// # Arguments
    /// * `jwt_token` - Raw JWT attestation token from Confidential Space
    /// * `expected_audience` - Expected audience claim
    /// * `evm_public_key` - 64-byte uncompressed secp256k1 public key
    /// * `cosign_signature` - 64-byte cosign signature (r || s) over the image digest
    /// * `cosign_pubkey` - 64 or 65 byte cosign public key (P-256 / secp256r1)
    pub async fn generate_proof(
        &self,
        jwt_token: &str,
        expected_audience: &str,
        evm_public_key: &[u8; 64],
        cosign_signature: &[u8],
        cosign_pubkey: &[u8],
    ) -> Result<ProofResponse, BootstrapError> {
        // Check for mock mode
        if self.service_url.starts_with("mock://") {
            return self.generate_mock_proof(evm_public_key);
        }

        let request = ProofRequest {
            jwt_token: jwt_token.to_string(),
            expected_audience: expected_audience.to_string(),
            evm_public_key: format!("0x{}", hex::encode(evm_public_key)),
            cosign_signature: format!("0x{}", hex::encode(cosign_signature)),
            cosign_pubkey: format!("0x{}", hex::encode(cosign_pubkey)),
        };

        info!(
            service_url = %self.service_url,
            timeout_secs = self.timeout.as_secs(),
            "Requesting proof generation"
        );

        let response = self
            .client
            .post(format!("{}/prove", self.service_url))
            .json(&request)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    BootstrapError::ProofGenerationTimeout(self.timeout)
                } else if e.is_connect() {
                    BootstrapError::ProofServiceUnavailable(e.to_string())
                } else {
                    BootstrapError::ProofGenerationFailed(e.to_string())
                }
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(BootstrapError::ProofGenerationFailed(format!(
                "HTTP {status}: {body}"
            )));
        }

        let proof_response: ProofResponse = response
            .json()
            .await
            .map_err(|e| BootstrapError::ProofGenerationFailed(e.to_string()))?;

        info!(
            tee_address = %proof_response.tee_address,
            "Proof generation complete"
        );

        Ok(proof_response)
    }

    /// Check if the proof service is healthy
    pub async fn health_check(&self) -> Result<bool, BootstrapError> {
        if self.service_url.starts_with("mock://") {
            return Ok(true);
        }

        let response = self
            .client
            .get(format!("{}/health", self.service_url))
            .timeout(self.health_check_timeout)
            .send()
            .await
            .map_err(|e| BootstrapError::ProofServiceUnavailable(e.to_string()))?;

        Ok(response.status().is_success())
    }

    /// Generate a mock proof for testing
    fn generate_mock_proof(
        &self,
        evm_public_key: &[u8; 64],
    ) -> Result<ProofResponse, BootstrapError> {
        warn!("Using MOCK prover - proofs will NOT be valid on-chain");

        // Derive address from public key (same as EvmKeyManager)
        let hash = alloy::primitives::keccak256(evm_public_key);
        let address = Address::from_slice(&hash[12..]);

        debug!(address = %address, "Generated mock proof");

        // Build ABI-encoded public values with correct tee_address placement
        // PublicValuesStruct has 12 fields × 32 bytes = 384 bytes ABI-encoded
        // Slot 4 (bytes 128-160): tee_signing_key (address is right-aligned, bytes 140-160)
        let mut public_values_bytes = vec![0u8; 384];
        // Place address at bytes 140-160 (right-aligned in 32-byte slot 4)
        public_values_bytes[140..160].copy_from_slice(address.as_slice());

        Ok(ProofResponse {
            public_values: format!("0x{}", hex::encode(&public_values_bytes)),
            proof_bytes: "0x".into(),
            tee_address: format!("{address}"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mock_prover() {
        let config = BootstrapConfig {
            prover_mode: ProverMode::Mock,
            ..Default::default()
        };

        let client = ProofClient::from_config(&config).unwrap();
        assert!(client.service_url.starts_with("mock://"));
    }
}
