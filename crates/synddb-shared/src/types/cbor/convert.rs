//! Conversion from CBOR types to internal message types
//!
//! This module provides conversion functions to convert CBOR/COSE messages
//! to the internal `SignedMessage` and `SignedBatch` types used by validators.

use super::{
    batch::CborBatch,
    error::CborError,
    message::{CborMessageType, CborSignedMessage},
    verify::verifying_key_from_bytes,
};
use crate::types::message::{MessageType, SignedBatch, SignedMessage};
use k256::ecdsa::VerifyingKey;

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
    /// Convert to the internal `SignedMessage` format.
    ///
    /// This parses the COSE structure, extracts all fields, and verifies the signature.
    /// The COSE protected header is included for signature verification.
    ///
    /// # Arguments
    /// * `expected_pubkey` - The expected signer's public key
    ///
    /// # Returns
    /// The converted `SignedMessage` that can be independently verified.
    pub fn to_signed_message(
        &self,
        expected_pubkey: &VerifyingKey,
    ) -> Result<SignedMessage, CborError> {
        // Parse and verify the COSE structure
        let parsed = self.verify_and_parse(expected_pubkey)?;

        // Extract the protected header for re-verification by validator
        let cose_protected_header = self.protected_header()?;

        // Convert message type
        let message_type: MessageType = parsed.message_type.into();

        // Format signature as 64-byte hex (r || s, no recovery byte)
        let signature_hex = format!("0x{}", hex::encode(parsed.signature));

        // Format message hash from payload
        let message_hash = format!("0x{}", hex::encode(compute_message_hash(&parsed.payload)));

        // Format public key as hex
        let signer = format!("0x{}", hex::encode(parsed.pubkey));

        Ok(SignedMessage {
            sequence: parsed.sequence,
            timestamp: parsed.timestamp,
            message_type,
            payload: parsed.payload,
            message_hash,
            signature: signature_hex,
            signer,
            cose_protected_header,
        })
    }

    /// Convert to `SignedMessage` without signature verification.
    ///
    /// WARNING: This does not verify the signature. Only use for debugging/inspection.
    /// The COSE protected header is still included so the caller can verify later.
    pub fn to_signed_message_unchecked(&self) -> Result<SignedMessage, CborError> {
        let parsed = self.parse_without_verify()?;

        // Still extract protected header for potential later verification
        let cose_protected_header = self.protected_header()?;

        let message_type: MessageType = parsed.message_type.into();

        // 64-byte signature (r || s)
        let signature_hex = format!("0x{}", hex::encode(parsed.signature));

        let message_hash = format!("0x{}", hex::encode(compute_message_hash(&parsed.payload)));
        let signer = format!("0x{}", hex::encode(parsed.pubkey));

        Ok(SignedMessage {
            sequence: parsed.sequence,
            timestamp: parsed.timestamp,
            message_type,
            payload: parsed.payload,
            message_hash,
            signature: signature_hex,
            signer,
            cose_protected_header,
        })
    }
}

impl CborBatch {
    /// Convert to the internal `SignedBatch` format.
    ///
    /// This verifies all COSE signatures during conversion.
    ///
    /// # Returns
    /// The converted `SignedBatch` if all verifications succeed.
    pub fn to_signed_batch(&self) -> Result<SignedBatch, CborError> {
        // Verify batch signature first (COSE verification)
        self.verify_batch_signature()?;

        // Convert pubkey bytes to VerifyingKey once for all messages
        let verifying_key = verifying_key_from_bytes(&self.pubkey)?;

        // Convert all messages (each verifies its COSE signature)
        let messages: Result<Vec<SignedMessage>, CborError> = self
            .messages
            .iter()
            .map(|m| m.to_signed_message(&verifying_key))
            .collect();
        let messages = messages?;

        // Format batch signature as 64-byte hex (r || s)
        let batch_signature = format!("0x{}", hex::encode(self.batch_signature));

        // Format public key as hex
        let signer = format!("0x{}", hex::encode(self.pubkey));

        Ok(SignedBatch {
            start_sequence: self.start_sequence,
            end_sequence: self.end_sequence,
            messages,
            batch_signature,
            signer,
            created_at: self.created_at,
            content_hash: self.content_hash,
        })
    }

    /// Convert to `SignedBatch` without full signature verification.
    ///
    /// This only parses the batch structure without verifying signatures.
    /// The COSE protected header is still included so the caller can verify later.
    pub fn to_signed_batch_unchecked(&self) -> Result<SignedBatch, CborError> {
        let messages: Result<Vec<SignedMessage>, CborError> = self
            .messages
            .iter()
            .map(|m| m.to_signed_message_unchecked())
            .collect();
        let messages = messages?;

        let batch_signature = format!("0x{}", hex::encode(self.batch_signature));
        let signer = format!("0x{}", hex::encode(self.pubkey));

        Ok(SignedBatch {
            start_sequence: self.start_sequence,
            end_sequence: self.end_sequence,
            messages,
            batch_signature,
            signer,
            created_at: self.created_at,
            content_hash: self.content_hash,
        })
    }
}

/// Compute message hash (keccak256 of payload)
fn compute_message_hash(payload: &[u8]) -> [u8; 32] {
    use alloy::primitives::keccak256;
    keccak256(payload).0
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
