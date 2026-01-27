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
    /// TEE public key (64-byte uncompressed, hex-encoded)
    pub tee_public_key: String,
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
        })
    }

    /// Generate a proof for the given attestation
    pub async fn generate_proof(
        &self,
        jwt_token: &str,
        expected_audience: &str,
        tee_public_key: &[u8; 64],
    ) -> Result<ProofResponse, BootstrapError> {
        // Check for mock mode
        if self.service_url.starts_with("mock://") {
            return self.generate_mock_proof(tee_public_key);
        }

        let request = ProofRequest {
            jwt_token: jwt_token.to_string(),
            expected_audience: expected_audience.to_string(),
            tee_public_key: format!("0x{}", hex::encode(tee_public_key)),
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
            .timeout(Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| BootstrapError::ProofServiceUnavailable(e.to_string()))?;

        Ok(response.status().is_success())
    }

    /// Generate a mock proof for testing
    fn generate_mock_proof(
        &self,
        tee_public_key: &[u8; 64],
    ) -> Result<ProofResponse, BootstrapError> {
        warn!("Using MOCK prover - proofs will NOT be valid on-chain");

        // Derive address from public key (same as EvmKeyManager)
        let hash = alloy::primitives::keccak256(tee_public_key);
        let address = Address::from_slice(&hash[12..]);

        debug!(address = %address, "Generated mock proof");

        // Return mock data that matches expected format
        Ok(ProofResponse {
            // Mock public values - 256 bytes of zeros
            public_values: format!("0x{}", "00".repeat(256)),
            // Mock proof - empty
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
