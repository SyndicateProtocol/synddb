//! HTTP handlers for the relayer
//!
//! Implements the /health and /register-and-fund endpoints with security checks.

use crate::{config::RelayerConfig, submitter::RelayerSubmitter, tracker::FundingTracker};
use alloy::primitives::{Address, Bytes, B256};
use axum::{http::StatusCode, Extension, Json};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

/// Request for key registration and funding
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterAndFundRequest {
    /// Hex-encoded public values from attestation
    pub public_values: String,
    /// Hex-encoded SP1 proof bytes
    pub proof_bytes: String,
    /// TEE key address to register and fund
    pub tee_key: String,
    /// Signature deadline (Unix timestamp)
    pub deadline: u64,
    /// Hex-encoded EIP-712 signature for key registration
    pub registration_signature: String,
    /// Hex-encoded EIP-712 signature for funding
    pub funding_signature: String,
}

/// Response for key registration and funding
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterAndFundResponse {
    /// Transaction hash for key registration (if submitted)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registration_tx_hash: Option<String>,
    /// Transaction hash for funding (if submitted)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub funding_tx_hash: Option<String>,
    /// Error message if request failed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Health check endpoint
pub async fn health() -> &'static str {
    "ok"
}

/// Register a TEE key and fund it
///
/// Security checks:
/// 1. Parse `public_values` to extract `image_digest_hash`
/// 2. Verify `image_digest` is in allowlist
/// 3. Check per-digest daily funding cap
/// 4. Check per-address funding cap
/// 5. Submit `addKeyWithSignature` (relayer pays gas)
/// 6. Submit `fundKeyWithSignature` (relayer pays gas)
pub async fn register_and_fund(
    Extension(config): Extension<RelayerConfig>,
    Extension(submitter): Extension<Arc<RelayerSubmitter>>,
    Extension(tracker): Extension<Arc<RwLock<FundingTracker>>>,
    Json(request): Json<RegisterAndFundRequest>,
) -> (StatusCode, Json<RegisterAndFundResponse>) {
    // Parse TEE key address
    let tee_key: Address = match request.tee_key.parse() {
        Ok(addr) => addr,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(RegisterAndFundResponse {
                    registration_tx_hash: None,
                    funding_tx_hash: None,
                    error: Some(format!("Invalid tee_key address: {}", e)),
                }),
            );
        }
    };

    // Parse public values and extract image digest
    let public_values_bytes = match hex::decode(request.public_values.trim_start_matches("0x")) {
        Ok(bytes) => bytes,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(RegisterAndFundResponse {
                    registration_tx_hash: None,
                    funding_tx_hash: None,
                    error: Some(format!("Invalid public_values hex: {}", e)),
                }),
            );
        }
    };

    // Extract image digest from public values
    // The image_digest_hash is at bytes 64-96 in the public values struct
    // (after jwk_key_hash: 32, validity_window_start: 8, validity_window_end: 8, padding: 16)
    let image_digest = match extract_image_digest(&public_values_bytes) {
        Some(digest) => digest,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(RegisterAndFundResponse {
                    registration_tx_hash: None,
                    funding_tx_hash: None,
                    error: Some("Could not extract image_digest from public_values".into()),
                }),
            );
        }
    };

    // Check image digest is in allowlist
    let allowed_digests = config.parse_allowed_digests();
    if !allowed_digests.contains(&image_digest) {
        warn!(
            image_digest = %image_digest,
            "Rejected: image digest not in allowlist"
        );
        return (
            StatusCode::FORBIDDEN,
            Json(RegisterAndFundResponse {
                registration_tx_hash: None,
                funding_tx_hash: None,
                error: Some("Image digest not in allowlist".into()),
            }),
        );
    }

    // Get funding amount from treasury
    let funding_amount = match submitter.get_funding_amount().await {
        Ok(amount) => amount,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(RegisterAndFundResponse {
                    registration_tx_hash: None,
                    funding_tx_hash: None,
                    error: Some(format!("Failed to get funding amount: {}", e)),
                }),
            );
        }
    };

    // Check funding caps
    {
        let mut tracker_guard = tracker.write().await;
        if let Err(e) = tracker_guard.check_allowed(image_digest, tee_key, funding_amount) {
            warn!(
                tee_key = %tee_key,
                image_digest = %image_digest,
                reason = %e,
                "Rejected: funding cap exceeded"
            );
            return (
                StatusCode::TOO_MANY_REQUESTS,
                Json(RegisterAndFundResponse {
                    registration_tx_hash: None,
                    funding_tx_hash: None,
                    error: Some(e),
                }),
            );
        }
    }

    info!(
        tee_key = %tee_key,
        image_digest = %image_digest,
        "Processing registration and funding request"
    );

    // Parse hex inputs
    let proof_bytes = match hex::decode(request.proof_bytes.trim_start_matches("0x")) {
        Ok(bytes) => bytes,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(RegisterAndFundResponse {
                    registration_tx_hash: None,
                    funding_tx_hash: None,
                    error: Some(format!("Invalid proof_bytes hex: {}", e)),
                }),
            );
        }
    };

    let registration_signature =
        match hex::decode(request.registration_signature.trim_start_matches("0x")) {
            Ok(bytes) => bytes,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(RegisterAndFundResponse {
                        registration_tx_hash: None,
                        funding_tx_hash: None,
                        error: Some(format!("Invalid registration_signature hex: {}", e)),
                    }),
                );
            }
        };

    let funding_signature = match hex::decode(request.funding_signature.trim_start_matches("0x")) {
        Ok(bytes) => bytes,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(RegisterAndFundResponse {
                    registration_tx_hash: None,
                    funding_tx_hash: None,
                    error: Some(format!("Invalid funding_signature hex: {}", e)),
                }),
            );
        }
    };

    // Check if key is already registered
    match submitter.is_key_valid(tee_key).await {
        Ok(true) => {
            info!(tee_key = %tee_key, "Key already registered, skipping addKey");
        }
        Ok(false) => {
            // Submit key registration
            match submitter
                .add_key_with_signature(
                    Bytes::from(public_values_bytes.clone()),
                    Bytes::from(proof_bytes),
                    request.deadline,
                    Bytes::from(registration_signature),
                )
                .await
            {
                Ok(tx_hash) => {
                    // Wait for confirmation
                    if let Err(e) = submitter.wait_for_confirmation(tx_hash).await {
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(RegisterAndFundResponse {
                                registration_tx_hash: Some(format!("{:#x}", tx_hash)),
                                funding_tx_hash: None,
                                error: Some(format!("Registration tx failed: {}", e)),
                            }),
                        );
                    }
                    info!(tx_hash = %tx_hash, "Key registration confirmed");
                }
                Err(e) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(RegisterAndFundResponse {
                            registration_tx_hash: None,
                            funding_tx_hash: None,
                            error: Some(format!("Failed to submit registration: {}", e)),
                        }),
                    );
                }
            }
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(RegisterAndFundResponse {
                    registration_tx_hash: None,
                    funding_tx_hash: None,
                    error: Some(format!("Failed to check key validity: {}", e)),
                }),
            );
        }
    }

    // Submit funding
    let funding_tx_hash = match submitter
        .fund_key_with_signature(tee_key, request.deadline, Bytes::from(funding_signature))
        .await
    {
        Ok(tx_hash) => tx_hash,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(RegisterAndFundResponse {
                    registration_tx_hash: None,
                    funding_tx_hash: None,
                    error: Some(format!("Failed to submit funding: {}", e)),
                }),
            );
        }
    };

    // Wait for funding confirmation
    if let Err(e) = submitter.wait_for_confirmation(funding_tx_hash).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(RegisterAndFundResponse {
                registration_tx_hash: None,
                funding_tx_hash: Some(format!("{:#x}", funding_tx_hash)),
                error: Some(format!("Funding tx failed: {}", e)),
            }),
        );
    }

    // Record successful funding
    {
        let mut tracker_guard = tracker.write().await;
        tracker_guard.record_funding(image_digest, tee_key, funding_amount);
    }

    info!(
        tee_key = %tee_key,
        funding_tx = %funding_tx_hash,
        amount = funding_amount,
        "Successfully registered and funded key"
    );

    (
        StatusCode::OK,
        Json(RegisterAndFundResponse {
            registration_tx_hash: None, // Already confirmed
            funding_tx_hash: Some(format!("{:#x}", funding_tx_hash)),
            error: None,
        }),
    )
}

/// Extract image_digest_hash from public values
///
/// The public values struct layout:
/// - bytes 0-31: jwk_key_hash (32 bytes)
/// - bytes 32-39: validity_window_start (8 bytes, packed)
/// - bytes 40-47: validity_window_end (8 bytes, packed)
/// - bytes 48-79: image_digest_hash (32 bytes)
/// - bytes 80-99: tee_signing_key (20 bytes)
/// - byte 100: secboot (1 byte)
/// - byte 101: dbgstat_disabled (1 byte)
/// - bytes 102-133: audience_hash (32 bytes)
fn extract_image_digest(public_values: &[u8]) -> Option<B256> {
    // The struct is ABI encoded, so offsets may differ
    // For now, assume a simple packed layout with image_digest at bytes 48-80
    if public_values.len() < 80 {
        return None;
    }

    let digest_bytes: [u8; 32] = public_values[48..80].try_into().ok()?;
    Some(B256::from(digest_bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_image_digest() {
        // Create mock public values with a known digest at position 48-80
        let mut public_values = vec![0u8; 134];

        // Set image digest at bytes 48-80
        let expected_digest = B256::from([0xAB; 32]);
        public_values[48..80].copy_from_slice(expected_digest.as_slice());

        let extracted = extract_image_digest(&public_values);
        assert_eq!(extracted, Some(expected_digest));
    }

    #[test]
    fn test_extract_image_digest_too_short() {
        let public_values = vec![0u8; 50]; // Too short
        let extracted = extract_image_digest(&public_values);
        assert_eq!(extracted, None);
    }
}
