//! CBOR message types

use super::{cose_helpers, error::CborError};
use serde::{Deserialize, Serialize};

/// Message type as integer for compact CBOR encoding
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CborMessageType {
    /// `SQLite` changeset batch
    Changeset = 0,
    /// Withdrawal request
    Withdrawal = 1,
    /// Database snapshot
    Snapshot = 2,
}

impl From<crate::types::message::MessageType> for CborMessageType {
    fn from(mt: crate::types::message::MessageType) -> Self {
        match mt {
            crate::types::message::MessageType::Changeset => Self::Changeset,
            crate::types::message::MessageType::Withdrawal => Self::Withdrawal,
            crate::types::message::MessageType::Snapshot => Self::Snapshot,
        }
    }
}

impl CborMessageType {
    /// Convert from u8 value
    pub const fn from_u8(value: u8) -> Result<Self, CborError> {
        match value {
            0 => Ok(Self::Changeset),
            1 => Ok(Self::Withdrawal),
            2 => Ok(Self::Snapshot),
            _ => Err(CborError::InvalidMessageType(value)),
        }
    }

    /// Convert to u8 value
    pub const fn as_u8(self) -> u8 {
        self as u8
    }
}

/// Parsed contents of a `COSE_Sign1` message
#[derive(Debug, Clone)]
pub struct ParsedCoseMessage {
    /// Sequence number
    pub sequence: u64,
    /// Unix timestamp
    pub timestamp: u64,
    /// Message type
    pub message_type: CborMessageType,
    /// Payload (zstd-compressed inner data)
    pub payload: Vec<u8>,
    /// 64-byte signature (r || s, without recovery byte)
    pub signature: [u8; 64],
    /// 64-byte uncompressed public key (without 0x04 prefix)
    pub pubkey: [u8; 64],
}

/// CBOR-serialized signed message using `COSE_Sign1` structure
#[derive(Debug, Clone)]
pub struct CborSignedMessage {
    /// Raw `COSE_Sign1` bytes
    cose_bytes: Vec<u8>,
}

impl CborSignedMessage {
    /// Create and sign a new message
    ///
    /// # Arguments
    /// * `sequence` - Monotonic sequence number
    /// * `timestamp` - Unix timestamp
    /// * `message_type` - Type of message
    /// * `payload` - Already zstd-compressed payload bytes
    /// * `signer_pubkey` - 64-byte uncompressed public key (without 0x04 prefix)
    /// * `sign_fn` - Function to sign the COSE `Sig_structure`, returns 64-byte signature
    pub fn new<F>(
        sequence: u64,
        timestamp: u64,
        message_type: CborMessageType,
        payload: Vec<u8>,
        signer_pubkey: [u8; 64],
        sign_fn: F,
    ) -> Result<Self, CborError>
    where
        F: FnOnce(&[u8]) -> Result<[u8; 64], CborError>,
    {
        let cose_bytes = cose_helpers::build_cose_sign1(
            sequence,
            timestamp,
            message_type,
            payload,
            signer_pubkey,
            sign_fn,
        )?;

        Ok(Self { cose_bytes })
    }

    /// Create from raw `COSE_Sign1` bytes (no validation)
    pub const fn from_bytes(bytes: Vec<u8>) -> Self {
        Self { cose_bytes: bytes }
    }

    /// Get raw `COSE_Sign1` bytes for storage
    pub fn as_bytes(&self) -> &[u8] {
        &self.cose_bytes
    }

    /// Consume and return the raw bytes
    pub fn into_bytes(self) -> Vec<u8> {
        self.cose_bytes
    }

    /// Verify signature and parse contents
    ///
    /// Returns the parsed message if signature verification succeeds.
    pub fn verify_and_parse(
        &self,
        expected_pubkey: &[u8; 64],
    ) -> Result<ParsedCoseMessage, CborError> {
        cose_helpers::verify_and_parse_cose_sign1(&self.cose_bytes, expected_pubkey)
    }

    /// Parse without verification (for debugging only)
    ///
    /// WARNING: This does not verify the signature. Only use for debugging/inspection.
    pub fn parse_without_verify(&self) -> Result<ParsedCoseMessage, CborError> {
        cose_helpers::parse_cose_sign1(&self.cose_bytes)
    }

    /// Get sequence without full verification (for indexing)
    pub fn sequence(&self) -> Result<u64, CborError> {
        cose_helpers::extract_sequence(&self.cose_bytes)
    }

    /// Get the size of the serialized message in bytes
    pub const fn size(&self) -> usize {
        self.cose_bytes.len()
    }

    /// Extract the CBOR-encoded protected header.
    ///
    /// This is needed for COSE signature verification by the validator.
    /// The protected header contains the sequence, timestamp, and message type.
    pub fn protected_header(&self) -> Result<Vec<u8>, CborError> {
        cose_helpers::extract_protected_header(&self.cose_bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_type_roundtrip() {
        assert_eq!(
            CborMessageType::from_u8(0).unwrap(),
            CborMessageType::Changeset
        );
        assert_eq!(
            CborMessageType::from_u8(1).unwrap(),
            CborMessageType::Withdrawal
        );
        assert_eq!(
            CborMessageType::from_u8(2).unwrap(),
            CborMessageType::Snapshot
        );
        assert!(CborMessageType::from_u8(3).is_err());
    }

    #[test]
    fn test_message_type_as_u8() {
        assert_eq!(CborMessageType::Changeset.as_u8(), 0);
        assert_eq!(CborMessageType::Withdrawal.as_u8(), 1);
        assert_eq!(CborMessageType::Snapshot.as_u8(), 2);
    }
}
