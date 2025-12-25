//! Core message types for validator wire format
//!
//! All messages use CBOR/COSE binary format. The types in this module are used
//! as internal representations after parsing from CBOR.

use crate::types::cbor::verify::{
    signature_from_bytes, verify_secp256k1, verifying_key_from_bytes,
};
use alloy::primitives::keccak256;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Errors that can occur during signature verification
#[derive(Debug, Error)]
pub enum VerificationError {
    #[error("Invalid signature format: {0}")]
    InvalidSignature(String),

    #[error("Invalid public key format: {0}")]
    InvalidPublicKey(String),

    #[error("Invalid message hash format: {0}")]
    InvalidHash(String),

    #[error("Signature verification failed: {0}")]
    VerificationFailed(String),

    #[error("Public key mismatch: expected {expected}, got {actual}")]
    PublicKeyMismatch { expected: String, actual: String },

    #[error("Serialization failed: {0}")]
    SerializationFailed(String),

    #[error(
        "Header mismatch: {field} in message ({outer}) differs from protected header ({header})"
    )]
    HeaderMismatch {
        field: String,
        outer: String,
        header: String,
    },

    #[error("Protected header parse error: {0}")]
    ProtectedHeaderParse(String),
}

/// Message types that can be validated
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MessageType {
    /// SQLite changeset batch
    Changeset,
    /// Withdrawal request to be processed on L1
    Withdrawal,
    /// Database snapshot
    Snapshot,
}

/// A message that has been validated and signed by a validator.
///
/// This is the internal representation after parsing from CBOR/COSE format.
/// All messages use `COSE_Sign1` signatures with 64-byte (r || s) format.
///
/// # Signature Verification
///
/// The signature is a `COSE_Sign1` signature computed over the COSE `Sig_structure`.
/// The protected header contains the sequence, timestamp, and message type.
///
/// Call [`SignedMessage::verify_signature()`] to verify the signature.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedMessage {
    /// Monotonically increasing sequence number
    pub sequence: u64,
    /// Unix timestamp (seconds) when sequenced
    pub timestamp: u64,
    /// Type of message
    pub message_type: MessageType,
    /// Compressed message payload (`zstd`-compressed)
    pub payload: Vec<u8>,
    /// Hash of the compressed payload: `keccak256(compressed_payload)`
    pub message_hash: String,
    /// Signature bytes (64 bytes, r || s). Encoded as hex with 0x prefix.
    pub signature: String,
    /// 64-byte uncompressed public key (without 0x04 prefix). Encoded as hex with 0x prefix.
    pub signer: String,
    /// CBOR-encoded COSE protected header for signature verification
    pub cose_protected_header: Vec<u8>,
}

impl SignedMessage {
    /// Verify that the signature is valid and was created by the claimed signer.
    ///
    /// Uses COSE `Sig_structure` for verification:
    /// `["Signature1", protected_header, external_aad, payload]`
    ///
    /// This also validates that the outer message fields (sequence, timestamp)
    /// match the values in the protected header, preventing field substitution attacks.
    pub fn verify_signature(&self) -> Result<(), VerificationError> {
        // Parse and validate protected header fields match outer fields
        let (header_sequence, header_timestamp) =
            parse_cose_protected_header_fields(&self.cose_protected_header)?;

        if self.sequence != header_sequence {
            return Err(VerificationError::HeaderMismatch {
                field: "sequence".to_string(),
                outer: self.sequence.to_string(),
                header: header_sequence.to_string(),
            });
        }

        if self.timestamp != header_timestamp {
            return Err(VerificationError::HeaderMismatch {
                field: "timestamp".to_string(),
                outer: self.timestamp.to_string(),
                header: header_timestamp.to_string(),
            });
        }

        // Build the COSE Sig_structure that was signed
        // Format: ["Signature1", protected, external_aad, payload]
        let sig_structure = build_cose_sig_structure(&self.cose_protected_header, &self.payload);

        // Parse signature (64 bytes, no recovery id for COSE)
        let sig_hex = self.signature.strip_prefix("0x").unwrap_or(&self.signature);
        let sig_bytes = hex::decode(sig_hex)
            .map_err(|e| VerificationError::InvalidSignature(format!("Invalid hex: {e}")))?;

        if sig_bytes.len() != 64 {
            return Err(VerificationError::InvalidSignature(format!(
                "COSE signature must be 64 bytes, got {}",
                sig_bytes.len()
            )));
        }

        // Parse the claimed public key (64 bytes)
        let pubkey_hex = self.signer.strip_prefix("0x").unwrap_or(&self.signer);
        let pubkey_bytes = hex::decode(pubkey_hex)
            .map_err(|e| VerificationError::InvalidPublicKey(format!("Invalid hex: {e}")))?;

        if pubkey_bytes.len() != 64 {
            return Err(VerificationError::InvalidPublicKey(format!(
                "Public key must be 64 bytes, got {}",
                pubkey_bytes.len()
            )));
        }

        let signature_array: [u8; 64] = sig_bytes
            .try_into()
            .map_err(|_| VerificationError::InvalidSignature("Invalid signature length".into()))?;

        let signature = signature_from_bytes(&signature_array)
            .map_err(|e| VerificationError::InvalidSignature(e.to_string()))?;

        let pubkey_array: [u8; 64] = pubkey_bytes
            .try_into()
            .map_err(|_| VerificationError::InvalidPublicKey("Invalid public key length".into()))?;

        // Convert to VerifyingKey for type-safe verification
        let verifying_key = verifying_key_from_bytes(&pubkey_array)
            .map_err(|e| VerificationError::InvalidPublicKey(e.to_string()))?;

        // Use the consolidated verify module (hashes with keccak256 internally)
        verify_secp256k1(&sig_structure, &signature, &verifying_key)
            .map_err(|e| VerificationError::VerificationFailed(e.to_string()))
    }
}

/// A batch of signed messages for atomic publication.
///
/// This is the internal representation after parsing from CBOR format.
/// All batches use CBOR encoding with 64-byte COSE signatures.
///
/// # Batch Signature Verification
///
/// The batch signature is computed over: `keccak256(keccak256(start || end || content_hash))`
/// where `content_hash` is the SHA-256 hash of all CBOR-encoded messages.
///
/// Call [`SignedBatch::verify_batch_signature()`] to verify the batch signature,
/// or [`SignedBatch::verify_all_signatures()`] to verify both batch and message signatures.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedBatch {
    /// First sequence number in this batch (inclusive)
    pub start_sequence: u64,
    /// Last sequence number in this batch (inclusive)
    pub end_sequence: u64,
    /// The messages in this batch, ordered by sequence number
    pub messages: Vec<SignedMessage>,
    /// Batch signature (64 bytes, r || s). Encoded as hex with 0x prefix.
    pub batch_signature: String,
    /// 64-byte uncompressed public key (without 0x04 prefix). Encoded as hex with 0x prefix.
    pub signer: String,
    /// Unix timestamp (seconds) when the batch was created
    pub created_at: u64,
    /// SHA-256 content hash of all CBOR-encoded messages
    pub content_hash: [u8; 32],
}

impl SignedBatch {
    /// Verify that the batch signature is valid and was created by the claimed signer.
    ///
    /// This does NOT verify individual message signatures - call
    /// `verify_all_signatures()` for complete verification.
    pub fn verify_batch_signature(&self) -> Result<(), VerificationError> {
        // Parse the claimed public key (64 bytes)
        let pubkey_hex = self.signer.strip_prefix("0x").unwrap_or(&self.signer);
        let pubkey_bytes = hex::decode(pubkey_hex)
            .map_err(|e| VerificationError::InvalidPublicKey(format!("Invalid hex: {e}")))?;

        if pubkey_bytes.len() != 64 {
            return Err(VerificationError::InvalidPublicKey(format!(
                "Public key must be 64 bytes, got {}",
                pubkey_bytes.len()
            )));
        }

        // Compute the signing payload: keccak256(start || end || content_hash)
        let mut data = Vec::with_capacity(8 + 8 + 32);
        data.extend_from_slice(&self.start_sequence.to_be_bytes());
        data.extend_from_slice(&self.end_sequence.to_be_bytes());
        data.extend_from_slice(&self.content_hash);
        let signing_payload = keccak256(&data);

        // Parse 64-byte signature
        let sig_hex = self
            .batch_signature
            .strip_prefix("0x")
            .unwrap_or(&self.batch_signature);
        let sig_bytes = hex::decode(sig_hex)
            .map_err(|e| VerificationError::InvalidSignature(format!("Invalid hex: {e}")))?;

        if sig_bytes.len() != 64 {
            return Err(VerificationError::InvalidSignature(format!(
                "Batch signature must be 64 bytes, got {}",
                sig_bytes.len()
            )));
        }

        let signature_array: [u8; 64] = sig_bytes
            .try_into()
            .map_err(|_| VerificationError::InvalidSignature("Invalid signature length".into()))?;

        let signature = signature_from_bytes(&signature_array)
            .map_err(|e| VerificationError::InvalidSignature(e.to_string()))?;

        let pubkey_array: [u8; 64] = pubkey_bytes
            .try_into()
            .map_err(|_| VerificationError::InvalidPublicKey("Invalid public key length".into()))?;

        // Convert to VerifyingKey for type-safe verification
        let verifying_key = verifying_key_from_bytes(&pubkey_array)
            .map_err(|e| VerificationError::InvalidPublicKey(e.to_string()))?;

        // The signature is over keccak256(signing_payload)
        // verify_secp256k1 hashes its input with keccak256, so we pass signing_payload
        verify_secp256k1(signing_payload.as_slice(), &signature, &verifying_key)
            .map_err(|e| VerificationError::VerificationFailed(e.to_string()))
    }

    /// Verify the batch signature and all individual message signatures.
    ///
    /// This is the complete verification that a validator should perform.
    pub fn verify_all_signatures(&self) -> Result<(), VerificationError> {
        // Verify batch signature
        self.verify_batch_signature()?;

        // Verify each message signature
        for msg in &self.messages {
            msg.verify_signature()?;
        }

        Ok(())
    }
}

// ============================================================================
// Helper functions for COSE signature verification
// ============================================================================

/// Build the COSE `Sig_structure` for signature verification.
///
/// The `Sig_structure` is a CBOR array: `["Signature1", protected, external_aad, payload]`
/// where:
/// - "Signature1" is the context string
/// - protected is the CBOR-encoded protected header
/// - `external_aad` is empty (we use `b""`)
/// - payload is the message payload
pub fn build_cose_sig_structure(protected_header: &[u8], payload: &[u8]) -> Vec<u8> {
    use ciborium::Value;

    let sig_structure = Value::Array(vec![
        Value::Text("Signature1".to_string()),
        Value::Bytes(protected_header.to_vec()),
        Value::Bytes(vec![]), // external_aad is empty
        Value::Bytes(payload.to_vec()),
    ]);

    let mut buf = Vec::new();
    ciborium::into_writer(&sig_structure, &mut buf).expect("CBOR serialization should not fail");
    buf
}

/// Parse sequence and timestamp from a COSE protected header.
///
/// The protected header is CBOR-encoded and contains custom fields:
/// - Sequence: label -65537
/// - Timestamp: label -65538
pub fn parse_cose_protected_header_fields(
    protected_header: &[u8],
) -> Result<(u64, u64), VerificationError> {
    use ciborium::Value;

    // Parse the CBOR map
    let header: Value = ciborium::from_reader(protected_header)
        .map_err(|e| VerificationError::ProtectedHeaderParse(format!("CBOR parse error: {e}")))?;

    let map = match header {
        Value::Map(m) => m,
        _ => {
            return Err(VerificationError::ProtectedHeaderParse(
                "Protected header is not a CBOR map".to_string(),
            ))
        }
    };

    // Custom header labels (from cose_helpers.rs)
    const HEADER_SEQUENCE: i128 = -65537;
    const HEADER_TIMESTAMP: i128 = -65538;

    let mut sequence: Option<u64> = None;
    let mut timestamp: Option<u64> = None;

    for (key, value) in map {
        if let Value::Integer(label) = key {
            let label_i128: i128 = label.into();
            if label_i128 == HEADER_SEQUENCE {
                if let Value::Integer(v) = value {
                    let v_i128: i128 = v.into();
                    sequence = Some(v_i128 as u64);
                }
            } else if label_i128 == HEADER_TIMESTAMP {
                if let Value::Integer(v) = value {
                    let v_i128: i128 = v.into();
                    timestamp = Some(v_i128 as u64);
                }
            }
        }
    }

    let sequence = sequence.ok_or_else(|| {
        VerificationError::ProtectedHeaderParse("Missing sequence in protected header".to_string())
    })?;

    let timestamp = timestamp.ok_or_else(|| {
        VerificationError::ProtectedHeaderParse("Missing timestamp in protected header".to_string())
    })?;

    Ok((sequence, timestamp))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_type_serialization() {
        // Verify tagged enum serialization matches expected format
        let changeset = MessageType::Changeset;
        let json = serde_json::to_string(&changeset).unwrap();
        assert_eq!(json, r#"{"type":"changeset"}"#);

        let withdrawal = MessageType::Withdrawal;
        let json = serde_json::to_string(&withdrawal).unwrap();
        assert_eq!(json, r#"{"type":"withdrawal"}"#);

        let snapshot = MessageType::Snapshot;
        let json = serde_json::to_string(&snapshot).unwrap();
        assert_eq!(json, r#"{"type":"snapshot"}"#);
    }

    #[test]
    fn test_message_type_deserialization() {
        let changeset: MessageType = serde_json::from_str(r#"{"type":"changeset"}"#).unwrap();
        assert_eq!(changeset, MessageType::Changeset);
    }

    #[test]
    fn test_signed_message_serialization() {
        let msg = SignedMessage {
            sequence: 42,
            timestamp: 1700000000,
            message_type: MessageType::Changeset,
            payload: vec![1, 2, 3],
            message_hash: "0xabc".to_string(),
            signature: "0xdef".to_string(),
            signer: "0x123".to_string(),
            cose_protected_header: vec![0xa0], // Empty CBOR map
        };

        let json = serde_json::to_string(&msg).unwrap();
        let decoded: SignedMessage = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded.sequence, 42);
        assert_eq!(decoded.timestamp, 1700000000);
        assert_eq!(decoded.payload, vec![1, 2, 3]);
    }

    #[test]
    fn test_signed_batch_serialization() {
        let messages = vec![
            SignedMessage {
                sequence: 1,
                timestamp: 1700000000,
                message_type: MessageType::Changeset,
                payload: vec![1, 2, 3],
                message_hash: "0xabc".to_string(),
                signature: "0xdef".to_string(),
                signer: "0x123".to_string(),
                cose_protected_header: vec![0xa0],
            },
            SignedMessage {
                sequence: 2,
                timestamp: 1700000001,
                message_type: MessageType::Withdrawal,
                payload: vec![4, 5, 6],
                message_hash: "0xghi".to_string(),
                signature: "0xjkl".to_string(),
                signer: "0x123".to_string(),
                cose_protected_header: vec![0xa0],
            },
        ];

        let batch = SignedBatch {
            start_sequence: 1,
            end_sequence: 2,
            messages,
            batch_signature: "0xbatchsig".to_string(),
            signer: "0x123".to_string(),
            created_at: 1700000002,
            content_hash: [0x42u8; 32],
        };

        let json = serde_json::to_string(&batch).unwrap();
        let decoded: SignedBatch = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded.start_sequence, 1);
        assert_eq!(decoded.end_sequence, 2);
        assert_eq!(decoded.messages.len(), 2);
        assert_eq!(decoded.messages[0].sequence, 1);
        assert_eq!(decoded.messages[1].sequence, 2);
    }

    #[test]
    fn test_parse_cose_protected_header_fields() {
        use ciborium::Value;

        // Custom header labels (must match the constants in parse_cose_protected_header_fields)
        const HEADER_SEQUENCE: i64 = -65537;
        const HEADER_TIMESTAMP: i64 = -65538;

        // Build a valid protected header
        let header = Value::Map(vec![
            (
                Value::Integer(HEADER_SEQUENCE.into()),
                Value::Integer(42i64.into()),
            ),
            (
                Value::Integer(HEADER_TIMESTAMP.into()),
                Value::Integer(1700000000i64.into()),
            ),
        ]);
        let mut encoded = Vec::new();
        ciborium::into_writer(&header, &mut encoded).unwrap();

        let (sequence, timestamp) = parse_cose_protected_header_fields(&encoded).unwrap();
        assert_eq!(sequence, 42);
        assert_eq!(timestamp, 1700000000);
    }

    #[test]
    fn test_parse_cose_protected_header_missing_sequence() {
        use ciborium::Value;

        const HEADER_TIMESTAMP: i64 = -65538;

        let header = Value::Map(vec![(
            Value::Integer(HEADER_TIMESTAMP.into()),
            Value::Integer(1700000000i64.into()),
        )]);
        let mut encoded = Vec::new();
        ciborium::into_writer(&header, &mut encoded).unwrap();

        let result = parse_cose_protected_header_fields(&encoded);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Missing sequence"));
    }

    #[test]
    fn test_parse_cose_protected_header_missing_timestamp() {
        use ciborium::Value;

        const HEADER_SEQUENCE: i64 = -65537;

        let header = Value::Map(vec![(
            Value::Integer(HEADER_SEQUENCE.into()),
            Value::Integer(42i64.into()),
        )]);
        let mut encoded = Vec::new();
        ciborium::into_writer(&header, &mut encoded).unwrap();

        let result = parse_cose_protected_header_fields(&encoded);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Missing timestamp"));
    }

    #[test]
    fn test_parse_cose_protected_header_invalid_cbor() {
        let invalid = vec![0xff, 0xff, 0xff];
        let result = parse_cose_protected_header_fields(&invalid);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("CBOR parse error"));
    }

    #[test]
    fn test_parse_cose_protected_header_not_a_map() {
        use ciborium::Value;

        // Encode an array instead of a map
        let not_a_map = Value::Array(vec![Value::Integer(42i64.into())]);
        let mut encoded = Vec::new();
        ciborium::into_writer(&not_a_map, &mut encoded).unwrap();

        let result = parse_cose_protected_header_fields(&encoded);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not a CBOR map"));
    }
}
