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
    /// This parses the COSE structure, extracts all fields, and includes the
    /// COSE protected header for later verification. The signature is stored
    /// as 64 bytes (r || s) without recovery byte, since COSE verification
    /// tries both recovery IDs.
    ///
    /// # Arguments
    /// * `expected_signer` - 20-byte Ethereum address to verify against
    ///
    /// # Returns
    /// The converted `SignedMessage` with `cose_protected_header` set.
    /// The validator can independently verify the signature using the
    /// protected header to reconstruct the COSE `Sig_structure`.
    pub fn to_signed_message(
        &self,
        expected_signer: &[u8; 20],
    ) -> Result<SignedMessage, CborError> {
        // Parse and verify the COSE structure
        let parsed = self.verify_and_parse(expected_signer)?;

        // Extract the protected header for re-verification by validator
        let cose_protected_header = self.protected_header()?;

        // Convert message type
        let message_type: MessageType = parsed.message_type.into();

        // Format signature as 64-byte hex (r || s, no recovery byte)
        // Validator will try both recovery IDs when verifying
        let signature_hex = format!("0x{}", hex::encode(parsed.signature));

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
            cose_protected_header: Some(cose_protected_header),
        })
    }

    /// Convert to `SignedMessage` without signature verification.
    ///
    /// WARNING: This does not verify the signature. Only use for debugging/inspection.
    /// The `cose_protected_header` is still included so the validator can verify later.
    pub fn to_signed_message_unchecked(&self) -> Result<SignedMessage, CborError> {
        let parsed = self.parse_without_verify()?;

        // Still extract protected header for potential later verification
        let cose_protected_header = self.protected_header()?;

        let message_type: MessageType = parsed.message_type.into();

        // 64-byte signature (r || s)
        let signature_hex = format!("0x{}", hex::encode(parsed.signature));

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
            cose_protected_header: Some(cose_protected_header),
        })
    }
}

impl CborBatch {
    /// Convert to the JSON-compatible `SignedBatch` format.
    ///
    /// This verifies all COSE signatures during conversion and includes the
    /// `cose_protected_header` in each message so validators can independently
    /// re-verify the signatures.
    ///
    /// # Returns
    /// The converted `SignedBatch` if all verifications succeed.
    pub fn to_signed_batch(&self) -> Result<SignedBatch, CborError> {
        // Verify batch signature first (COSE verification)
        self.verify_batch_signature()?;

        // Convert all messages (each verifies its COSE signature)
        let messages: Result<Vec<SignedMessage>, CborError> = self
            .messages
            .iter()
            .map(|m| m.to_signed_message(&self.signer))
            .collect();
        let messages = messages?;

        // Format batch signature as 64-byte hex (r || s)
        let batch_signature = format!("0x{}", hex::encode(self.batch_signature));

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
    /// This only parses the batch structure without verifying signatures.
    /// The `cose_protected_header` is still included so the validator can verify later.
    pub fn to_signed_batch_unchecked(&self) -> Result<SignedBatch, CborError> {
        let messages: Result<Vec<SignedMessage>, CborError> = self
            .messages
            .iter()
            .map(|m| m.to_signed_message_unchecked())
            .collect();
        let messages = messages?;

        let batch_signature = format!("0x{}", hex::encode(self.batch_signature));
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
