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

/// Extract attestation fields from ABI-encoded public values
///
/// The `PublicValuesStruct` is ABI-encoded by the SP1 program, meaning each field
/// occupies a 32-byte slot regardless of its actual size:
///
/// | Slot | Bytes     | Field                    | Notes                           |
/// |------|-----------|--------------------------|-------------------------------- |
/// | 0    | 0-31      | `jwk_key_hash`           | bytes32                         |
/// | 1    | 32-63     | `validity_window_start`  | uint64, right-aligned           |
/// | 2    | 64-95     | `validity_window_end`    | uint64, right-aligned           |
/// | 3    | 96-127    | `image_digest_hash`      | bytes32                         |
/// | 4    | 128-159   | `tee_signing_key`        | address, right-aligned (140-159)|
/// | 5    | 160-191   | `secboot`                | bool, right-aligned             |
/// | 6    | 192-223   | `dbgstat_disabled`       | bool, right-aligned             |
/// | 7    | 224-255   | `audience_hash`          | bytes32                         |
/// | 8    | 256-287   | `cosign_signature_r`     | bytes32                         |
/// | 9    | 288-319   | `cosign_signature_s`     | bytes32                         |
/// | 10   | 320-351   | `cosign_pubkey_x`        | bytes32                         |
/// | 11   | 352-383   | `cosign_pubkey_y`        | bytes32                         |
///
/// Total: 384 bytes (12 slots × 32 bytes)
fn extract_attestation_fields(public_values: &[u8]) -> Option<AttestationFields> {
    // ABI-encoded struct is 12 slots × 32 bytes = 384 bytes
    if public_values.len() < 384 {
        return None;
    }

    // Slot 3: image_digest_hash (bytes32, full slot)
    let image_digest: [u8; 32] = public_values[96..128].try_into().ok()?;

    // Slot 4: tee_signing_key (address is 20 bytes, right-aligned in 32-byte slot)
    let tee_signing_key: [u8; 20] = public_values[140..160].try_into().ok()?;

    // Slot 7: audience_hash (bytes32, full slot)
    let audience_hash: [u8; 32] = public_values[224..256].try_into().ok()?;

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
        // Create mock ABI-encoded public values (12 slots × 32 bytes = 384 bytes)
        let mut public_values = vec![0u8; 384];

        // Slot 3 (bytes 96-128): image_digest_hash
        let expected_digest = B256::from([0xAB; 32]);
        public_values[96..128].copy_from_slice(expected_digest.as_slice());

        // Slot 4 (bytes 128-160): tee_signing_key (address is right-aligned, bytes 140-160)
        let expected_key = alloy::primitives::Address::from([0xCD; 20]);
        public_values[140..160].copy_from_slice(expected_key.as_slice());

        // Slot 7 (bytes 224-256): audience_hash
        let expected_audience = B256::from([0xEF; 32]);
        public_values[224..256].copy_from_slice(expected_audience.as_slice());

        let extracted = extract_attestation_fields(&public_values);
        assert!(extracted.is_some());

        let fields = extracted.unwrap();
        assert_eq!(fields.image_digest, expected_digest);
        assert_eq!(fields.tee_signing_key, expected_key);
        assert_eq!(fields.audience_hash, expected_audience);
    }

    #[test]
    fn test_extract_attestation_fields_too_short() {
        let public_values = vec![0u8; 300]; // Too short (needs 384 bytes)
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
