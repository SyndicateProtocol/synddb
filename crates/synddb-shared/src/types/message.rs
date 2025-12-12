//! Core message types for sequencer wire format

use super::serde_helpers::{base64_serde, base64_serde_opt};
use alloy::{
    primitives::{keccak256, Address, B256},
    signers::Signature,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Errors that can occur during signature verification
#[derive(Debug, Error)]
pub enum VerificationError {
    #[error("Invalid signature format: {0}")]
    InvalidSignature(String),

    #[error("Invalid signer address format: {0}")]
    InvalidAddress(String),

    #[error("Invalid message hash format: {0}")]
    InvalidHash(String),

    #[error("Signature recovery failed: {0}")]
    RecoveryFailed(String),

    #[error("Signer mismatch: expected {expected}, got {actual}")]
    SignerMismatch { expected: String, actual: String },

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

/// Message types that can be sequenced
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MessageType {
    /// `SQLite` changeset batch from `synddb-client`
    Changeset,
    /// Withdrawal request to be processed on L1
    Withdrawal,
    /// Database snapshot from `synddb-client`
    Snapshot,
}

/// A message that has been sequenced and signed by the sequencer.
///
/// # Signature Formats
///
/// This struct supports two signature formats:
///
/// ## Legacy (JSON) Format
/// The signature is computed over: `keccak256(sequence || timestamp || message_hash)`
/// where:
/// - `sequence` is 8 bytes, big-endian
/// - `timestamp` is 8 bytes, big-endian
/// - `message_hash` is 32 bytes (the keccak256 of the compressed payload)
///
/// ## COSE Format
/// When `cose_protected_header` is present, the signature is a `COSE_Sign1` signature
/// computed over the COSE `Sig_structure`. The protected header contains the sequence,
/// timestamp, and message type. This format is used for CBOR-encoded batches.
///
/// # Verification
///
/// Call [`SignedMessage::verify_signature()`] which automatically detects the format:
/// - If `cose_protected_header` is present: verifies using COSE `Sig_structure`
/// - Otherwise: verifies using the legacy format
///
/// # JSON Serialization
///
/// ```json
/// {
///   "sequence": 42,
///   "timestamp": 1700000000,
///   "message_type": {"type": "changeset"},
///   "payload": "KLUv/QAAA...",
///   "message_hash": "0x1234abcd...",
///   "signature": "0xabcd1234...(130 hex chars)...",
///   "signer": "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"
/// }
/// ```
///
/// Note: `payload` is serialized as a base64-encoded string for compactness,
/// and `message_type` is a tagged enum.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedMessage {
    /// Monotonically increasing sequence number
    pub sequence: u64,
    /// Unix timestamp (seconds) when sequenced
    pub timestamp: u64,
    /// Type of message
    pub message_type: MessageType,
    /// Compressed message payload (`zstd`-compressed JSON), base64-encoded in JSON
    #[serde(with = "base64_serde")]
    pub payload: Vec<u8>,
    /// Hash of the compressed payload: `keccak256(compressed_payload)`
    pub message_hash: String,
    /// Signature bytes (64 bytes for COSE, 65 bytes for legacy).
    /// Encoded as hex with 0x prefix.
    pub signature: String,
    /// Ethereum address of the signer (checksummed, with 0x prefix)
    pub signer: String,
    /// CBOR-encoded COSE protected header (if COSE signature format).
    ///
    /// When present, indicates the signature is a `COSE_Sign1` signature and verification
    /// should use the COSE `Sig_structure`. When absent, legacy verification is used.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "base64_serde_opt"
    )]
    pub cose_protected_header: Option<Vec<u8>>,
}

impl SignedMessage {
    /// Create the signing payload for a message (legacy format).
    ///
    /// Format: `keccak256(sequence || timestamp || message_hash)`
    /// where all values are big-endian encoded.
    pub fn compute_signing_payload(sequence: u64, timestamp: u64, message_hash: B256) -> B256 {
        let mut data = Vec::with_capacity(48);
        data.extend_from_slice(&sequence.to_be_bytes());
        data.extend_from_slice(&timestamp.to_be_bytes());
        data.extend_from_slice(message_hash.as_slice());
        keccak256(&data)
    }

    /// Verify that the signature is valid and was created by the claimed signer.
    ///
    /// Automatically detects the signature format:
    /// - If `cose_protected_header` is present: verifies using COSE `Sig_structure`
    /// - Otherwise: verifies using the legacy format
    pub fn verify_signature(&self) -> Result<(), VerificationError> {
        self.cose_protected_header.as_ref().map_or_else(
            || self.verify_legacy_signature(),
            |protected_header| self.verify_cose_signature(protected_header),
        )
    }

    /// Verify using legacy format: signature over `keccak256(sequence || timestamp || message_hash)`
    fn verify_legacy_signature(&self) -> Result<(), VerificationError> {
        // Parse the message hash
        let message_hash = parse_b256(&self.message_hash)?;

        // Compute the signing payload
        let payload = Self::compute_signing_payload(self.sequence, self.timestamp, message_hash);

        // Parse and verify the signature (65 bytes with recovery id)
        let recovered_address = recover_signer(&self.signature, payload)?;

        // Parse the claimed signer
        let claimed_signer: Address = self
            .signer
            .parse()
            .map_err(|e| VerificationError::InvalidAddress(format!("{e}")))?;

        // Compare addresses
        if recovered_address != claimed_signer {
            return Err(VerificationError::SignerMismatch {
                expected: format!("{claimed_signer:?}"),
                actual: format!("{recovered_address:?}"),
            });
        }

        Ok(())
    }

    /// Verify using COSE format: signature over COSE `Sig_structure`
    ///
    /// The `Sig_structure` is: `["Signature1", protected_header, external_aad, payload]`
    ///
    /// This function also validates that the outer message fields (sequence, timestamp)
    /// match the values in the protected header, preventing field substitution attacks.
    fn verify_cose_signature(&self, protected_header: &[u8]) -> Result<(), VerificationError> {
        // Parse and validate protected header fields match outer fields
        let (header_sequence, header_timestamp) =
            parse_cose_protected_header_fields(protected_header)?;

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
        let sig_structure = build_cose_sig_structure(protected_header, &self.payload);

        // Hash it with keccak256 (Ethereum style)
        let message_hash = keccak256(&sig_structure);

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

        // Parse the claimed signer
        let claimed_signer: Address = self
            .signer
            .parse()
            .map_err(|e| VerificationError::InvalidAddress(format!("{e}")))?;

        // Try both recovery IDs since COSE doesn't store v
        let signature_array: [u8; 64] = sig_bytes
            .try_into()
            .map_err(|_| VerificationError::InvalidSignature("Invalid signature length".into()))?;

        verify_secp256k1_without_recovery_id(&message_hash, &signature_array, &claimed_signer)
    }
}

/// A batch of signed messages for atomic publication.
///
/// Batches combine multiple messages into a single atomic unit for publication
/// to data availability layers. This ensures that messages and state are always
/// published together, preventing partial publication failures.
///
/// # Batch Signature Verification
///
/// The batch signature is computed over: `keccak256(start_sequence || end_sequence || messages_hash)`
/// where:
/// - `start_sequence` is 8 bytes, big-endian
/// - `end_sequence` is 8 bytes, big-endian
/// - `messages_hash` is `keccak256(json(messages))` using **compact JSON** (no whitespace)
///
/// To verify:
/// 1. Call [`SignedBatch::verify_batch_signature()`] to verify the batch signature
/// 2. Call [`SignedBatch::verify_all_signatures()`] to verify both the batch and all message signatures
///
/// # JSON Serialization Format
///
/// Batches are serialized as pretty-printed JSON for storage:
///
/// ```json
/// {
///   "start_sequence": 1,
///   "end_sequence": 1,
///   "messages": [
///     {
///       "sequence": 1,
///       "timestamp": 1700000000,
///       "message_type": {"type": "changeset"},
///       "payload": "KLUv/QAAA...",
///       "message_hash": "0x...",
///       "signature": "0x...",
///       "signer": "0x..."
///     }
///   ],
///   "batch_signature": "0x...(130 hex chars)...",
///   "signer": "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
///   "created_at": 1700000000
/// }
/// ```
///
/// **Important**: The `messages_hash` for signature verification is computed from
/// **compact JSON** (no whitespace), not the pretty-printed storage format:
/// ```text
/// [{"sequence":1,"timestamp":1700000000,"message_type":{"type":"changeset"},...}]
/// ```
///
/// # Storage Layout
///
/// Batches are stored with filenames indicating their sequence range:
/// ```text
/// {prefix}/batches/{start:012}_{end:012}.json
/// ```
///
/// For example: `batches/000000000001_000000000050.json` contains messages 1-50.
///
/// The filename encodes the state implicitly - the highest `end` sequence number
/// across all batch files represents the latest published state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedBatch {
    /// First sequence number in this batch (inclusive)
    pub start_sequence: u64,
    /// Last sequence number in this batch (inclusive)
    pub end_sequence: u64,
    /// The messages in this batch, ordered by sequence number
    pub messages: Vec<SignedMessage>,
    /// Signature over `keccak256(start_sequence || end_sequence || messages_hash)`.
    ///
    /// For legacy JSON format: 65 bytes (r || s || v), `messages_hash` is `keccak256(compact_json(messages))`
    /// For CBOR format: 64 bytes (r || s), `messages_hash` is the SHA-256 content hash stored in `cbor_content_hash`
    pub batch_signature: String,
    /// Ethereum address of the batch signer (checksummed, with 0x prefix)
    pub signer: String,
    /// Unix timestamp (seconds) when the batch was created
    pub created_at: u64,
    /// SHA-256 content hash from CBOR format (set when converted from `CborBatch`).
    /// When present, batch signature verification uses CBOR format (64-byte signature over content hash).
    /// When None, legacy JSON format is used (65-byte signature over JSON messages hash).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cbor_content_hash: Option<[u8; 32]>,
}

impl SignedBatch {
    /// Compute the hash of serialized messages for batch signing.
    ///
    /// The messages are serialized as compact JSON (no whitespace).
    pub fn compute_messages_hash(messages: &[SignedMessage]) -> Result<B256, VerificationError> {
        let json = serde_json::to_vec(messages)
            .map_err(|e| VerificationError::SerializationFailed(e.to_string()))?;
        Ok(keccak256(&json))
    }

    /// Create the signing payload for a batch.
    ///
    /// Format: `keccak256(start_sequence || end_sequence || messages_hash)`
    /// where all values are big-endian encoded.
    pub fn compute_signing_payload(
        start_sequence: u64,
        end_sequence: u64,
        messages_hash: B256,
    ) -> B256 {
        let mut data = Vec::with_capacity(48);
        data.extend_from_slice(&start_sequence.to_be_bytes());
        data.extend_from_slice(&end_sequence.to_be_bytes());
        data.extend_from_slice(messages_hash.as_slice());
        keccak256(&data)
    }

    /// Verify that the batch signature is valid and was created by the claimed signer.
    ///
    /// This does NOT verify individual message signatures - call
    /// `verify_all_signatures()` for complete verification.
    ///
    /// Supports both signature formats:
    /// - **CBOR format** (64 bytes): When `cbor_content_hash` is set, uses SHA-256 content hash
    /// - **Legacy JSON format** (65 bytes): When `cbor_content_hash` is None, uses keccak256 of JSON messages
    pub fn verify_batch_signature(&self) -> Result<(), VerificationError> {
        // Parse the claimed signer
        let claimed_signer: Address = self
            .signer
            .parse()
            .map_err(|e| VerificationError::InvalidAddress(format!("{e}")))?;

        // TODO(cleanup): Remove legacy JSON format detection once all clients use CBOR.
        // This auto-detection can be simplified to only support CBOR format.
        self.cbor_content_hash.as_ref().map_or_else(
            || self.verify_legacy_batch_signature(&claimed_signer),
            |content_hash| self.verify_cbor_batch_signature(content_hash, &claimed_signer),
        )
    }

    /// Verify batch signature using CBOR format (64-byte signature over content hash)
    fn verify_cbor_batch_signature(
        &self,
        content_hash: &[u8; 32],
        claimed_signer: &Address,
    ) -> Result<(), VerificationError> {
        // Compute the signing payload: keccak256(start || end || content_hash)
        // This matches CborBatch::compute_signing_payload()
        let mut data = Vec::with_capacity(8 + 8 + 32);
        data.extend_from_slice(&self.start_sequence.to_be_bytes());
        data.extend_from_slice(&self.end_sequence.to_be_bytes());
        data.extend_from_slice(content_hash);
        let signing_payload = keccak256(&data);

        // The signature is over keccak256(signing_payload), matching CborBatch::verify_batch_signature
        let message_hash = keccak256(signing_payload);

        // Parse 64-byte signature
        let sig_hex = self
            .batch_signature
            .strip_prefix("0x")
            .unwrap_or(&self.batch_signature);
        let sig_bytes = hex::decode(sig_hex)
            .map_err(|e| VerificationError::InvalidSignature(format!("Invalid hex: {e}")))?;

        if sig_bytes.len() != 64 {
            return Err(VerificationError::InvalidSignature(format!(
                "CBOR batch signature must be 64 bytes, got {}",
                sig_bytes.len()
            )));
        }

        let signature_array: [u8; 64] = sig_bytes
            .try_into()
            .map_err(|_| VerificationError::InvalidSignature("Invalid signature length".into()))?;

        verify_secp256k1_without_recovery_id(&message_hash, &signature_array, claimed_signer)
    }

    /// Verify batch signature using legacy JSON format (65-byte signature over JSON messages hash)
    ///
    /// TODO(cleanup): Remove this function once JSON format is deprecated
    fn verify_legacy_batch_signature(
        &self,
        claimed_signer: &Address,
    ) -> Result<(), VerificationError> {
        // Compute messages hash from the actual messages
        let messages_hash = Self::compute_messages_hash(&self.messages)?;

        // Compute the signing payload
        let payload =
            Self::compute_signing_payload(self.start_sequence, self.end_sequence, messages_hash);

        // Parse and verify the signature (65 bytes with recovery ID)
        let recovered_address = recover_signer(&self.batch_signature, payload)?;

        // Compare addresses
        if recovered_address != *claimed_signer {
            return Err(VerificationError::SignerMismatch {
                expected: format!("{claimed_signer:?}"),
                actual: format!("{recovered_address:?}"),
            });
        }

        Ok(())
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

/// Receipt returned to clients after successful sequencing
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SequenceReceipt {
    /// Assigned sequence number
    pub sequence: u64,
    /// Timestamp when sequenced
    pub timestamp: u64,
    /// Hash of the message payload
    pub message_hash: String,
    /// Signature proving the sequencer ordered this message
    pub signature: String,
    /// Address of the sequencer
    pub signer: String,
}

// ============================================================================
// Helper functions for signature verification
// ============================================================================

/// Parse a hex-encoded B256 hash (with or without 0x prefix)
fn parse_b256(hex_str: &str) -> Result<B256, VerificationError> {
    let hex_str = hex_str.strip_prefix("0x").unwrap_or(hex_str);
    let bytes = hex::decode(hex_str)
        .map_err(|e| VerificationError::InvalidHash(format!("Invalid hex: {e}")))?;
    if bytes.len() != 32 {
        return Err(VerificationError::InvalidHash(format!(
            "Expected 32 bytes, got {}",
            bytes.len()
        )));
    }
    Ok(B256::from_slice(&bytes))
}

/// Parse a hex-encoded signature and recover the signer address
fn recover_signer(signature_hex: &str, payload: B256) -> Result<Address, VerificationError> {
    let sig_hex = signature_hex.strip_prefix("0x").unwrap_or(signature_hex);
    let sig_bytes = hex::decode(sig_hex)
        .map_err(|e| VerificationError::InvalidSignature(format!("Invalid hex: {e}")))?;

    if sig_bytes.len() != 65 {
        return Err(VerificationError::InvalidSignature(format!(
            "Expected 65 bytes, got {}",
            sig_bytes.len()
        )));
    }

    // Parse r, s, v from the signature bytes
    let r = B256::from_slice(&sig_bytes[0..32]);
    let s = B256::from_slice(&sig_bytes[32..64]);
    let v = sig_bytes[64];

    // Convert v to parity (v=27/0 -> false, v=28/1 -> true)
    let parity = match v {
        27 | 0 => false,
        28 | 1 => true,
        _ => {
            return Err(VerificationError::InvalidSignature(format!(
                "Invalid v value: {v}"
            )))
        }
    };

    // Create signature and recover
    let signature = Signature::new(
        alloy::primitives::U256::from_be_bytes(r.0),
        alloy::primitives::U256::from_be_bytes(s.0),
        parity,
    );

    signature
        .recover_address_from_prehash(&payload)
        .map_err(|e| VerificationError::RecoveryFailed(format!("{e}")))
}

/// Build the COSE `Sig_structure` for signature verification.
///
/// The `Sig_structure` is a CBOR array: `["Signature1", protected, external_aad, payload]`
/// where:
/// - "Signature1" is the context string
/// - protected is the CBOR-encoded protected header
/// - `external_aad` is empty (we use `b""`)
/// - payload is the message payload
fn build_cose_sig_structure(protected_header: &[u8], payload: &[u8]) -> Vec<u8> {
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

/// Verify a secp256k1 signature without recovery ID by trying both possible values.
///
/// COSE signatures are 64 bytes (r || s) without the recovery ID (v).
/// We try both v=0 and v=1 to find which one recovers to the expected signer.
fn verify_secp256k1_without_recovery_id(
    message_hash: &B256,
    signature: &[u8; 64],
    expected_signer: &Address,
) -> Result<(), VerificationError> {
    use alloy::primitives::U256;

    let r = U256::from_be_slice(&signature[..32]);
    let s = U256::from_be_slice(&signature[32..]);

    // Try both recovery IDs
    for v in [false, true] {
        let sig = Signature::new(r, s, v);
        if let Ok(recovered) = sig.recover_address_from_prehash(message_hash) {
            if recovered == *expected_signer {
                return Ok(());
            }
        }
    }

    Err(VerificationError::RecoveryFailed(
        "Could not recover matching signer address from COSE signature".to_string(),
    ))
}

/// Parse sequence and timestamp from a COSE protected header.
///
/// The protected header is CBOR-encoded and contains custom fields:
/// - Sequence: label -65537
/// - Timestamp: label -65538
fn parse_cose_protected_header_fields(
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
            cose_protected_header: None,
        };

        let json = serde_json::to_string(&msg).unwrap();
        let decoded: SignedMessage = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded.sequence, 42);
        assert_eq!(decoded.timestamp, 1700000000);
        assert_eq!(decoded.payload, vec![1, 2, 3]);
        assert!(decoded.cose_protected_header.is_none());
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
                cose_protected_header: None,
            },
            SignedMessage {
                sequence: 2,
                timestamp: 1700000001,
                message_type: MessageType::Withdrawal,
                payload: vec![4, 5, 6],
                message_hash: "0xghi".to_string(),
                signature: "0xjkl".to_string(),
                signer: "0x123".to_string(),
                cose_protected_header: None,
            },
        ];

        let batch = SignedBatch {
            start_sequence: 1,
            end_sequence: 2,
            messages,
            batch_signature: "0xbatchsig".to_string(),
            signer: "0x123".to_string(),
            created_at: 1700000002,
            cbor_content_hash: None,
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
    fn test_batch_json_roundtrip_with_cbor_content_hash() {
        let content_hash = [0x42u8; 32];
        let batch = SignedBatch {
            start_sequence: 1,
            end_sequence: 2,
            messages: vec![],
            batch_signature: "0xbatchsig".to_string(),
            signer: "0x123".to_string(),
            created_at: 1700000002,
            cbor_content_hash: Some(content_hash),
        };

        let json = serde_json::to_string(&batch).unwrap();
        println!("JSON: {}", json);

        let decoded: SignedBatch = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.cbor_content_hash, Some(content_hash));
    }

    // ========================================================================
    // Signature verification tests
    // ========================================================================

    use alloy::signers::{local::PrivateKeySigner, Signer};

    // Test private key (Anvil default, DO NOT use in production!)
    // Address: 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266
    const TEST_PRIVATE_KEY: &str =
        "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

    /// Helper to sign a payload and return hex signature
    async fn sign_payload(signer: &PrivateKeySigner, payload: B256) -> String {
        let signature = signer.sign_hash(&payload).await.unwrap();
        let mut bytes = [0u8; 65];
        bytes[..32].copy_from_slice(&signature.r().to_be_bytes::<32>());
        bytes[32..64].copy_from_slice(&signature.s().to_be_bytes::<32>());
        bytes[64] = if signature.v() { 28 } else { 27 };
        format!("0x{}", hex::encode(bytes))
    }

    /// Create a properly signed message for testing
    async fn create_signed_message(
        signer: &PrivateKeySigner,
        sequence: u64,
        timestamp: u64,
    ) -> SignedMessage {
        let payload = b"test payload";
        let message_hash = keccak256(payload);
        let signing_payload =
            SignedMessage::compute_signing_payload(sequence, timestamp, message_hash);
        let signature = sign_payload(signer, signing_payload).await;

        SignedMessage {
            sequence,
            timestamp,
            message_type: MessageType::Changeset,
            payload: payload.to_vec(),
            message_hash: format!("0x{}", hex::encode(message_hash)),
            signature,
            signer: format!("{:?}", signer.address()),
            cose_protected_header: None,
        }
    }

    #[tokio::test]
    async fn test_message_signature_verification() {
        let signer: PrivateKeySigner = TEST_PRIVATE_KEY.parse().unwrap();
        let msg = create_signed_message(&signer, 42, 1700000000).await;

        // Valid signature should verify
        assert!(msg.verify_signature().is_ok());
    }

    #[tokio::test]
    async fn test_message_signature_wrong_signer() {
        let signer: PrivateKeySigner = TEST_PRIVATE_KEY.parse().unwrap();
        let mut msg = create_signed_message(&signer, 42, 1700000000).await;

        // Tamper with the signer address
        msg.signer = "0x0000000000000000000000000000000000000001".to_string();

        // Should fail with signer mismatch
        let result = msg.verify_signature();
        assert!(matches!(
            result,
            Err(VerificationError::SignerMismatch { .. })
        ));
    }

    #[tokio::test]
    async fn test_message_signature_tampered_sequence() {
        let signer: PrivateKeySigner = TEST_PRIVATE_KEY.parse().unwrap();
        let mut msg = create_signed_message(&signer, 42, 1700000000).await;

        // Tamper with the sequence (signature was for sequence 42)
        msg.sequence = 43;

        // Should fail - recovered signer won't match
        let result = msg.verify_signature();
        assert!(matches!(
            result,
            Err(VerificationError::SignerMismatch { .. })
        ));
    }

    #[tokio::test]
    async fn test_batch_signature_verification() {
        let signer: PrivateKeySigner = TEST_PRIVATE_KEY.parse().unwrap();
        let msg = create_signed_message(&signer, 1, 1700000000).await;

        // Create batch with proper signature
        let messages = vec![msg];
        let messages_hash = SignedBatch::compute_messages_hash(&messages).unwrap();
        let batch_payload = SignedBatch::compute_signing_payload(1, 1, messages_hash);
        let batch_signature = sign_payload(&signer, batch_payload).await;

        let batch = SignedBatch {
            start_sequence: 1,
            end_sequence: 1,
            messages,
            batch_signature,
            signer: format!("{:?}", signer.address()),
            created_at: 1700000000,
            cbor_content_hash: None, // TODO(cleanup): Remove when JSON format is deprecated
        };

        // Both batch and message signatures should verify
        assert!(batch.verify_batch_signature().is_ok());
        assert!(batch.verify_all_signatures().is_ok());
    }

    #[tokio::test]
    async fn test_batch_signature_tampered_message() {
        let signer: PrivateKeySigner = TEST_PRIVATE_KEY.parse().unwrap();
        let msg = create_signed_message(&signer, 1, 1700000000).await;

        // Create batch with proper signature
        let messages = vec![msg];
        let messages_hash = SignedBatch::compute_messages_hash(&messages).unwrap();
        let batch_payload = SignedBatch::compute_signing_payload(1, 1, messages_hash);
        let batch_signature = sign_payload(&signer, batch_payload).await;

        let mut batch = SignedBatch {
            start_sequence: 1,
            end_sequence: 1,
            messages,
            batch_signature,
            signer: format!("{:?}", signer.address()),
            created_at: 1700000000,
            cbor_content_hash: None, // TODO(cleanup): Remove when JSON format is deprecated
        };

        // Tamper with a message payload
        batch.messages[0].payload = b"tampered".to_vec();

        // Batch signature should fail (messages_hash changed)
        let result = batch.verify_batch_signature();
        assert!(matches!(
            result,
            Err(VerificationError::SignerMismatch { .. })
        ));
    }
}
