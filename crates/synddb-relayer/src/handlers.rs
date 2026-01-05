//! HTTP handlers for the relayer
//!
//! Implements the /health and /register-key endpoints.

use crate::{config::RelayerConfig, submitter::RelayerSubmitter};
use alloy::primitives::B256;
use axum::{http::StatusCode, Extension, Json};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{info, warn};

/// Key type for registration
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum KeyType {
    Sequencer,
    Validator,
}

/// Request for key registration
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RegisterKeyRequest {
    /// Hex-encoded public values from attestation
    pub public_values: String,
    /// Hex-encoded SP1 proof bytes
    pub proof_bytes: String,
    /// Signature deadline (Unix timestamp)
    pub deadline: u64,
    /// Hex-encoded EIP-712 signature for key registration
    pub signature: String,
    /// Type of key to register (sequencer or validator)
    pub key_type: KeyType,
}

/// Response for key registration
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RegisterKeyResponse {
    /// Registered key address (if successful)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registered_key: Option<String>,
    /// Transaction hash for key registration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx_hash: Option<String>,
    /// Error message if request failed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Health check endpoint
pub(crate) async fn health() -> &'static str {
    "ok"
}

/// Register a TEE key (sequencer or validator)
///
/// Security checks:
/// 1. Parse `public_values` to extract `image_digest_hash` and `audience_hash`
/// 2. Look up application config by `audience_hash`
/// 3. Verify `image_digest` is in that application's allowlist
/// 4. Submit key registration to Bridge (relayer pays gas)
pub(crate) async fn register_key(
    Extension(config): Extension<Arc<RelayerConfig>>,
    Extension(submitter): Extension<Arc<RelayerSubmitter>>,
    Json(request): Json<RegisterKeyRequest>,
) -> (StatusCode, Json<RegisterKeyResponse>) {
    // Parse public values
    let public_values_bytes = match hex::decode(request.public_values.trim_start_matches("0x")) {
        Ok(bytes) => bytes,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(RegisterKeyResponse {
                    registered_key: None,
                    tx_hash: None,
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
                Json(RegisterKeyResponse {
                    registered_key: None,
                    tx_hash: None,
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
                Json(RegisterKeyResponse {
                    registered_key: None,
                    tx_hash: None,
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
            Json(RegisterKeyResponse {
                registered_key: None,
                tx_hash: None,
                error: Some("Image digest not in allowlist for this application".into()),
            }),
        );
    }

    info!(
        tee_key = %attestation.tee_signing_key,
        key_type = ?request.key_type,
        image_digest = %attestation.image_digest,
        audience_hash = %attestation.audience_hash,
        "Processing key registration request"
    );

    // Parse hex inputs
    let proof_bytes = match hex::decode(request.proof_bytes.trim_start_matches("0x")) {
        Ok(bytes) => bytes,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(RegisterKeyResponse {
                    registered_key: None,
                    tx_hash: None,
                    error: Some(format!("Invalid proof_bytes hex: {}", e)),
                }),
            );
        }
    };

    let signature = match hex::decode(request.signature.trim_start_matches("0x")) {
        Ok(bytes) => bytes,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(RegisterKeyResponse {
                    registered_key: None,
                    tx_hash: None,
                    error: Some(format!("Invalid signature hex: {}", e)),
                }),
            );
        }
    };

    // Check if key is already registered
    let is_valid = match request.key_type {
        KeyType::Sequencer => {
            submitter
                .is_sequencer_key_valid(attestation.tee_signing_key)
                .await
        }
        KeyType::Validator => {
            submitter
                .is_validator_key_valid(attestation.tee_signing_key)
                .await
        }
    };

    match is_valid {
        Ok(true) => {
            info!(
                tee_key = %attestation.tee_signing_key,
                key_type = ?request.key_type,
                "Key already registered"
            );
            return (
                StatusCode::OK,
                Json(RegisterKeyResponse {
                    registered_key: Some(format!("{:#x}", attestation.tee_signing_key)),
                    tx_hash: None,
                    error: None,
                }),
            );
        }
        Ok(false) => {
            // Continue with registration
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(RegisterKeyResponse {
                    registered_key: None,
                    tx_hash: None,
                    error: Some(format!("Failed to check key validity: {}", e)),
                }),
            );
        }
    }

    // Submit key registration
    let result = match request.key_type {
        KeyType::Sequencer => {
            submitter
                .register_sequencer_key(
                    public_values_bytes,
                    proof_bytes,
                    request.deadline,
                    signature,
                )
                .await
        }
        KeyType::Validator => {
            submitter
                .register_validator_key(
                    public_values_bytes,
                    proof_bytes,
                    request.deadline,
                    signature,
                )
                .await
        }
    };

    match result {
        Ok(tx_hash) => {
            // Wait for confirmation
            if let Err(e) = submitter.wait_for_confirmation(tx_hash).await {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(RegisterKeyResponse {
                        registered_key: None,
                        tx_hash: Some(format!("{:#x}", tx_hash)),
                        error: Some(format!("Registration tx failed: {}", e)),
                    }),
                );
            }

            info!(
                tee_key = %attestation.tee_signing_key,
                tx_hash = %tx_hash,
                key_type = ?request.key_type,
                "Key registration confirmed"
            );

            (
                StatusCode::OK,
                Json(RegisterKeyResponse {
                    registered_key: Some(format!("{:#x}", attestation.tee_signing_key)),
                    tx_hash: Some(format!("{:#x}", tx_hash)),
                    error: None,
                }),
            )
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(RegisterKeyResponse {
                registered_key: None,
                tx_hash: None,
                error: Some(format!("Failed to submit registration: {}", e)),
            }),
        ),
    }
}

/// Extracted attestation fields from public values
struct AttestationFields {
    image_digest: B256,
    audience_hash: B256,
    tee_signing_key: alloy::primitives::Address,
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
    let tee_signing_key: [u8; 20] = public_values[80..100].try_into().ok()?;
    let audience_hash: [u8; 32] = public_values[102..134].try_into().ok()?;

    Some(AttestationFields {
        image_digest: B256::from(image_digest),
        tee_signing_key: alloy::primitives::Address::from(tee_signing_key),
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

        // Set TEE signing key at bytes 80-100
        let expected_key = alloy::primitives::Address::from([0xCD; 20]);
        public_values[80..100].copy_from_slice(expected_key.as_slice());

        // Set audience hash at bytes 102-134
        let expected_audience = B256::from([0xEF; 32]);
        public_values[102..134].copy_from_slice(expected_audience.as_slice());

        let extracted = extract_attestation_fields(&public_values);
        assert!(extracted.is_some());

        let fields = extracted.unwrap();
        assert_eq!(fields.image_digest, expected_digest);
        assert_eq!(fields.tee_signing_key, expected_key);
        assert_eq!(fields.audience_hash, expected_audience);
    }

    #[test]
    fn test_extract_attestation_fields_too_short() {
        let public_values = vec![0u8; 100]; // Too short
        let extracted = extract_attestation_fields(&public_values);
        assert!(extracted.is_none());
    }

    #[test]
    fn test_key_type_deserialize() {
        let json = r#""sequencer""#;
        let key_type: KeyType = serde_json::from_str(json).unwrap();
        assert_eq!(key_type, KeyType::Sequencer);

        let json = r#""validator""#;
        let key_type: KeyType = serde_json::from_str(json).unwrap();
        assert_eq!(key_type, KeyType::Validator);
    }
}
