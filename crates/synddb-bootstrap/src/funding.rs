//! Funding request client for signature-based gas provisioning
//!
//! When a new TEE key is generated without gas, it can request funding
//! from the relayer using an EIP-712 signature. The relayer verifies
//! the signature and submits the funding transaction.

use crate::{BootstrapError, ContractSubmitter};
use alloy::{
    primitives::{keccak256, Address, B256, U256},
    signers::{local::PrivateKeySigner, Signer},
};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, info};

/// Client for requesting funding from the relayer
pub struct FundingClient {
    relayer_url: String,
    treasury_address: Address,
    http_client: reqwest::Client,
}

impl std::fmt::Debug for FundingClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FundingClient")
            .field("relayer_url", &self.relayer_url)
            .field("treasury_address", &self.treasury_address)
            .finish_non_exhaustive()
    }
}

/// Request payload for funding
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FundingRequest {
    /// Public values from attestation proof
    pub public_values: String,
    /// SP1 proof bytes
    pub proof_bytes: String,
    /// Address of the TEE key to fund
    pub tee_key: String,
    /// Signature deadline (Unix timestamp)
    pub deadline: u64,
    /// EIP-712 signature for funding authorization
    pub funding_signature: String,
}

/// Response from the relayer
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FundingResponse {
    /// Transaction hash for key registration
    pub registration_tx_hash: Option<String>,
    /// Transaction hash for funding
    pub funding_tx_hash: Option<String>,
    /// Error message if request failed
    pub error: Option<String>,
}

/// EIP-712 domain for `GasTreasury`
struct Eip712Domain {
    name: &'static str,
    version: &'static str,
    chain_id: u64,
    verifying_contract: Address,
}

impl FundingClient {
    /// Create a new funding client
    pub fn new(relayer_url: String, treasury_address: Address) -> Self {
        Self {
            relayer_url,
            treasury_address,
            http_client: reqwest::Client::new(),
        }
    }

    /// Request funding from the relayer
    ///
    /// This function:
    /// 1. Creates an EIP-712 signature for the funding request
    /// 2. Sends the request to the relayer
    /// 3. Returns transaction hashes for registration and funding
    pub async fn request_funding(
        &self,
        signer: &PrivateKeySigner,
        public_values: &str,
        proof_bytes: &str,
        chain_id: u64,
        deadline: u64,
    ) -> Result<FundingResponse, BootstrapError> {
        let tee_key = signer.address();

        // Create EIP-712 signature
        let signature = self
            .create_funding_signature(signer, chain_id, deadline)
            .await?;

        let request = FundingRequest {
            public_values: public_values.to_string(),
            proof_bytes: proof_bytes.to_string(),
            tee_key: format!("{:#x}", tee_key),
            deadline,
            funding_signature: format!("0x{}", hex::encode(&signature)),
        };

        info!(
            relayer = %self.relayer_url,
            tee_key = %tee_key,
            deadline = deadline,
            "Requesting funding from relayer"
        );

        let url = format!("{}/register-and-fund", self.relayer_url);

        let response = self
            .http_client
            .post(&url)
            .json(&request)
            .timeout(Duration::from_secs(120))
            .send()
            .await
            .map_err(|e| {
                BootstrapError::ProofGenerationFailed(format!("Relayer request failed: {e}"))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(BootstrapError::ProofGenerationFailed(format!(
                "Relayer returned {}: {}",
                status, body
            )));
        }

        let funding_response: FundingResponse = response.json().await.map_err(|e| {
            BootstrapError::ProofGenerationFailed(format!("Invalid relayer response: {e}"))
        })?;

        if let Some(error) = &funding_response.error {
            return Err(BootstrapError::ProofGenerationFailed(format!(
                "Relayer error: {}",
                error
            )));
        }

        info!(
            registration_tx = ?funding_response.registration_tx_hash,
            funding_tx = ?funding_response.funding_tx_hash,
            "Funding request accepted"
        );

        Ok(funding_response)
    }

    /// Create EIP-712 signature for funding request
    async fn create_funding_signature(
        &self,
        signer: &PrivateKeySigner,
        chain_id: u64,
        deadline: u64,
    ) -> Result<Vec<u8>, BootstrapError> {
        let domain = Eip712Domain {
            name: "GasTreasury",
            version: "1",
            chain_id,
            verifying_contract: self.treasury_address,
        };

        // FundKey(address teeKey,uint256 nonce,uint256 deadline)
        // Note: nonce is 0 for new keys (never funded before)
        let nonce: u64 = 0;
        let tee_key = signer.address();

        // Compute domain separator
        let domain_separator = compute_domain_separator(&domain);

        // Compute struct hash
        let typehash = keccak256("FundKey(address teeKey,uint256 nonce,uint256 deadline)");
        let struct_hash = keccak256(
            [
                typehash.as_slice(),
                &[0u8; 12], // padding for address
                tee_key.as_slice(),
                &U256::from(nonce).to_be_bytes::<32>(),
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
            "Created EIP-712 digest for funding"
        );

        // Sign the digest
        let signature = signer
            .sign_hash(&digest)
            .await
            .map_err(|e| BootstrapError::Config(format!("Signing failed: {e}")))?;

        Ok(signature.as_bytes().to_vec())
    }

    /// Wait for funding to arrive at the TEE key address
    pub async fn wait_for_funding(
        &self,
        submitter: &ContractSubmitter,
        address: Address,
        min_balance: u128,
        timeout: Duration,
    ) -> Result<(), BootstrapError> {
        let start = std::time::Instant::now();
        let poll_interval = Duration::from_secs(2);

        loop {
            if start.elapsed() > timeout {
                return Err(BootstrapError::InsufficientBalance {
                    have: 0,
                    need: min_balance,
                });
            }

            let balance = submitter.get_balance(address).await?;

            if balance >= min_balance {
                info!(
                    address = %address,
                    balance_wei = balance,
                    "Funding received"
                );
                return Ok(());
            }

            debug!(
                address = %address,
                balance_wei = balance,
                min_balance_wei = min_balance,
                "Waiting for funding..."
            );

            tokio::time::sleep(poll_interval).await;
        }
    }
}

/// Compute EIP-712 domain separator
fn compute_domain_separator(domain: &Eip712Domain) -> B256 {
    let typehash = keccak256(
        "EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)",
    );

    keccak256(
        [
            typehash.as_slice(),
            keccak256(domain.name).as_slice(),
            keccak256(domain.version).as_slice(),
            &U256::from(domain.chain_id).to_be_bytes::<32>(),
            &[0u8; 12], // padding for address
            domain.verifying_contract.as_slice(),
        ]
        .concat(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_funding_request_serialization() {
        let request = FundingRequest {
            public_values: "0x1234".to_string(),
            proof_bytes: "0x5678".to_string(),
            tee_key: "0xabcd".to_string(),
            deadline: 12345,
            funding_signature: "0xsig".to_string(),
        };

        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("publicValues"));
        assert!(json.contains("proofBytes"));
        assert!(json.contains("teeKey"));
    }

    #[test]
    fn test_domain_separator_computation() {
        let domain = Eip712Domain {
            name: "GasTreasury",
            version: "1",
            chain_id: 1,
            verifying_contract: Address::ZERO,
        };

        let separator = compute_domain_separator(&domain);
        // Just verify it produces a valid hash
        assert_ne!(separator, B256::ZERO);
    }
}
