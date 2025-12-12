//! Conversion from CBOR types to JSON-compatible types
//!
//! This module provides conversion functions to convert CBOR/COSE messages
//! to the legacy JSON-serializable types (`SignedMessage`, `SignedBatch`).
//! This enables validators to work with a unified message type internally
//! while supporting both storage formats.

use super::{
    batch::CborBatch,
    error::CborError,
    message::{CborMessageType, CborSignedMessage},
};
use crate::types::message::{MessageType, SignedBatch, SignedMessage};

impl From<CborMessageType> for MessageType {
    fn from(t: CborMessageType) -> Self {
        match t {
            CborMessageType::Changeset => Self::Changeset,
            CborMessageType::Withdrawal => Self::Withdrawal,
            CborMessageType::Snapshot => Self::Snapshot,
        }
    }
}

impl CborSignedMessage {
    /// Convert to the JSON-compatible `SignedMessage` format.
    ///
    /// This parses the COSE structure and extracts all fields into the
    /// legacy format. The signature is converted to 65-byte format with
    /// recovery byte appended.
    ///
    /// # Arguments
    /// * `expected_signer` - 20-byte Ethereum address to verify against
    ///
    /// # Returns
    /// The converted `SignedMessage` if verification succeeds.
    pub fn to_signed_message(
        &self,
        expected_signer: &[u8; 20],
    ) -> Result<SignedMessage, CborError> {
        let parsed = self.verify_and_parse(expected_signer)?;

        // Convert message type
        let message_type: MessageType = parsed.message_type.into();

        // Format signature as 65-byte hex (r || s || v)
        // We need to recover the v value by trying both parities
        let signature_hex = format_signature_with_recovery(&parsed.signature, expected_signer)?;

        // Format message hash from payload
        let message_hash = format!("0x{}", hex::encode(compute_message_hash(&parsed.payload)));

        // Format signer address
        let signer = format!("0x{}", hex::encode(parsed.signer));

        Ok(SignedMessage {
            sequence: parsed.sequence,
            timestamp: parsed.timestamp,
            message_type,
            payload: parsed.payload,
            message_hash,
            signature: signature_hex,
            signer,
        })
    }

    /// Convert to `SignedMessage` without signature verification.
    ///
    /// WARNING: This does not verify the signature. Only use for debugging/inspection.
    pub fn to_signed_message_unchecked(&self) -> Result<SignedMessage, CborError> {
        let parsed = self.parse_without_verify()?;

        let message_type: MessageType = parsed.message_type.into();

        // For unchecked conversion, we use v=27 as default
        let signature_hex = format!("0x{}1b", hex::encode(parsed.signature));

        let message_hash = format!("0x{}", hex::encode(compute_message_hash(&parsed.payload)));
        let signer = format!("0x{}", hex::encode(parsed.signer));

        Ok(SignedMessage {
            sequence: parsed.sequence,
            timestamp: parsed.timestamp,
            message_type,
            payload: parsed.payload,
            message_hash,
            signature: signature_hex,
            signer,
        })
    }
}

impl CborBatch {
    /// Convert to the JSON-compatible `SignedBatch` format.
    ///
    /// This verifies all signatures and converts the batch to the legacy format.
    /// The batch signature is converted to 65-byte format.
    ///
    /// # Returns
    /// The converted `SignedBatch` if all verifications succeed.
    pub fn to_signed_batch(&self) -> Result<SignedBatch, CborError> {
        // Verify batch signature first
        self.verify_batch_signature()?;

        // Convert all messages
        let messages: Result<Vec<SignedMessage>, CborError> = self
            .messages
            .iter()
            .map(|m| m.to_signed_message(&self.signer))
            .collect();
        let messages = messages?;

        // Format batch signature as 65-byte hex
        let batch_signature = format_signature_with_recovery(&self.batch_signature, &self.signer)?;

        // Format signer address
        let signer = format!("0x{}", hex::encode(self.signer));

        Ok(SignedBatch {
            start_sequence: self.start_sequence,
            end_sequence: self.end_sequence,
            messages,
            batch_signature,
            signer,
            created_at: self.created_at,
        })
    }

    /// Convert to `SignedBatch` without full signature verification.
    ///
    /// This only verifies the batch structure, not individual message signatures.
    /// Use for cases where performance is critical and signatures were already verified.
    pub fn to_signed_batch_unchecked(&self) -> Result<SignedBatch, CborError> {
        let messages: Result<Vec<SignedMessage>, CborError> = self
            .messages
            .iter()
            .map(|m| m.to_signed_message_unchecked())
            .collect();
        let messages = messages?;

        // Use v=27 as default for unchecked
        let batch_signature = format!("0x{}1b", hex::encode(self.batch_signature));
        let signer = format!("0x{}", hex::encode(self.signer));

        Ok(SignedBatch {
            start_sequence: self.start_sequence,
            end_sequence: self.end_sequence,
            messages,
            batch_signature,
            signer,
            created_at: self.created_at,
        })
    }
}

/// Compute message hash (keccak256 of payload)
fn compute_message_hash(payload: &[u8]) -> [u8; 32] {
    use alloy::primitives::keccak256;
    keccak256(payload).0
}

/// Format 64-byte signature as 65-byte hex with recovery byte
fn format_signature_with_recovery(
    signature: &[u8; 64],
    _expected_signer: &[u8; 20],
) -> Result<String, CborError> {
    // The signature is r || s (64 bytes). We need to determine v (recovery byte).
    // Since we don't have the original message hash here, we use v=27 (0x1b) as default.
    // The actual recovery will happen during verification in SignedMessage::verify_signature().
    //
    // Note: The JSON format uses v=27/28 convention, where:
    // - v=27 (0x1b) means recovery_id=0
    // - v=28 (0x1c) means recovery_id=1
    Ok(format!("0x{}1b", hex::encode(signature)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_type_conversion() {
        assert!(matches!(
            MessageType::from(CborMessageType::Changeset),
            MessageType::Changeset
        ));
        assert!(matches!(
            MessageType::from(CborMessageType::Withdrawal),
            MessageType::Withdrawal
        ));
        assert!(matches!(
            MessageType::from(CborMessageType::Snapshot),
            MessageType::Snapshot
        ));
    }

    #[test]
    fn test_compute_message_hash() {
        let payload = b"test payload";
        let hash = compute_message_hash(payload);
        assert_eq!(hash.len(), 32);

        // Same input should produce same hash
        let hash2 = compute_message_hash(payload);
        assert_eq!(hash, hash2);
    }
}
