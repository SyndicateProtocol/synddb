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
pub(crate) struct RegisterAndFundRequest {
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
pub(crate) struct RegisterAndFundResponse {
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
pub(crate) async fn health() -> &'static str {
    "ok"
}

/// Register a TEE key and fund it
///
/// Security checks:
/// 1. Parse `public_values` to extract `image_digest_hash` and `audience_hash`
/// 2. Look up application config by `audience_hash`
/// 3. Verify `image_digest` is in that application's allowlist
/// 4. Check per-application funding caps (per-digest daily, per-address)
/// 5. Submit `addKeyWithSignature` (relayer pays gas)
/// 6. Submit `fundKeyWithSignature` to application's treasury (relayer pays gas)
pub(crate) async fn register_and_fund(
    Extension(config): Extension<Arc<RelayerConfig>>,
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

    // Parse public values
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

    // Extract attestation fields from public values
    let attestation = match extract_attestation_fields(&public_values_bytes) {
        Some(a) => a,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(RegisterAndFundResponse {
                    registration_tx_hash: None,
                    funding_tx_hash: None,
                    error: Some("Could not extract attestation fields from public_values".into()),
                }),
            );
        }
    };

    // Look up application by audience_hash
    let app_config = match config.get_application(&attestation.audience_hash) {
        Some(app) => app,
        None => {
            warn!(
                audience_hash = %attestation.audience_hash,
                "Rejected: unknown application (audience_hash not configured)"
            );
            return (
                StatusCode::FORBIDDEN,
                Json(RegisterAndFundResponse {
                    registration_tx_hash: None,
                    funding_tx_hash: None,
                    error: Some("Unknown application: audience_hash not configured".into()),
                }),
            );
        }
    };

    // Check image digest is in application's allowlist
    if !app_config
        .allowed_image_digests
        .contains(&attestation.image_digest)
    {
        warn!(
            image_digest = %attestation.image_digest,
            audience_hash = %attestation.audience_hash,
            "Rejected: image digest not in application's allowlist"
        );
        return (
            StatusCode::FORBIDDEN,
            Json(RegisterAndFundResponse {
                registration_tx_hash: None,
                funding_tx_hash: None,
                error: Some("Image digest not in allowlist for this application".into()),
            }),
        );
    }

    // Get funding amount from application's treasury
    let funding_amount = match submitter
        .get_funding_amount(app_config.treasury_address)
        .await
    {
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

    // Check per-application funding caps
    {
        let mut tracker_guard = tracker.write().await;
        if let Err(e) = tracker_guard.check_allowed(
            &attestation.audience_hash,
            &attestation.image_digest,
            tee_key,
            funding_amount,
            app_config,
        ) {
            warn!(
                tee_key = %tee_key,
                image_digest = %attestation.image_digest,
                audience_hash = %attestation.audience_hash,
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
        image_digest = %attestation.image_digest,
        audience_hash = %attestation.audience_hash,
        treasury = %app_config.treasury_address,
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

    // Submit funding to application's treasury
    let funding_tx_hash = match submitter
        .fund_key_with_signature(
            app_config.treasury_address,
            tee_key,
            request.deadline,
            Bytes::from(funding_signature),
        )
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
        tracker_guard.record_funding(
            &attestation.audience_hash,
            &attestation.image_digest,
            tee_key,
            funding_amount,
        );
    }

    info!(
        tee_key = %tee_key,
        funding_tx = %funding_tx_hash,
        amount = funding_amount,
        treasury = %app_config.treasury_address,
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

/// Extracted attestation fields from public values
struct AttestationFields {
    image_digest: B256,
    audience_hash: B256,
}

/// Extract attestation fields from public values
///
/// The public values struct layout:
/// - bytes 0-31: `jwk_key_hash` (32 bytes)
/// - bytes 32-39: `validity_window_start` (8 bytes, packed)
/// - bytes 40-47: `validity_window_end` (8 bytes, packed)
/// - bytes 48-79: `image_digest_hash` (32 bytes)
/// - bytes 80-99: `tee_signing_key` (20 bytes)
/// - byte 100: secboot (1 byte)
/// - byte 101: `dbgstat_disabled` (1 byte)
/// - bytes 102-133: `audience_hash` (32 bytes)
fn extract_attestation_fields(public_values: &[u8]) -> Option<AttestationFields> {
    // Need at least 134 bytes for the full struct
    if public_values.len() < 134 {
        return None;
    }

    let image_digest: [u8; 32] = public_values[48..80].try_into().ok()?;
    let audience_hash: [u8; 32] = public_values[102..134].try_into().ok()?;

    Some(AttestationFields {
        image_digest: B256::from(image_digest),
        audience_hash: B256::from(audience_hash),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_attestation_fields() {
        // Create mock public values with known values
        let mut public_values = vec![0u8; 134];

        // Set image digest at bytes 48-80
        let expected_digest = B256::from([0xAB; 32]);
        public_values[48..80].copy_from_slice(expected_digest.as_slice());

        // Set audience hash at bytes 102-134
        let expected_audience = B256::from([0xCD; 32]);
        public_values[102..134].copy_from_slice(expected_audience.as_slice());

        let extracted = extract_attestation_fields(&public_values);
        assert!(extracted.is_some());

        let fields = extracted.unwrap();
        assert_eq!(fields.image_digest, expected_digest);
        assert_eq!(fields.audience_hash, expected_audience);
    }

    #[test]
    fn test_extract_attestation_fields_too_short() {
        let public_values = vec![0u8; 100]; // Too short
        let extracted = extract_attestation_fields(&public_values);
        assert!(extracted.is_none());
    }
}
