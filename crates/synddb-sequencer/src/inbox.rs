//! Message inbox for ordering and sequencing
//!
//! Provides monotonic sequence number assignment for all incoming messages.
//! This is the core ordering mechanism similar to Arbitrum's delayed inbox.
//!
//! Messages are signed using COSE (CBOR Object Signing and Encryption) with
//! 64-byte secp256k1 ECDSA signatures (r || s format, no recovery ID).

use alloy::primitives::{keccak256, Address};
use k256::ecdsa::Signature;
use std::{
    io::Write,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{SystemTime, UNIX_EPOCH},
};
use synddb_shared::{
    keys::EvmKeyManager,
    types::cbor::{
        error::CborError,
        message::{CborMessageType, CborSignedMessage},
        verify::{signature_from_bytes, verifying_key_from_bytes},
    },
};
use thiserror::Error;

/// Errors from inbox operations
#[derive(Debug, Error)]
pub enum InboxError {
    #[error("Signing error: {0}")]
    Signing(String),

    #[error("CBOR encoding error: {0}")]
    Cbor(#[from] CborError),

    #[error("Invalid public key: {0}")]
    InvalidPublicKey(String),
}

/// Receipt returned to clients after successful sequencing
///
/// Contains the COSE signature (64 bytes, r || s format) for the sequenced message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SequenceReceipt {
    /// Assigned sequence number
    pub sequence: u64,
    /// Timestamp when sequenced
    pub timestamp: u64,
    /// Hash of the compressed payload: `keccak256(zstd_compressed_payload)`
    pub message_hash: String,
    /// COSE signature (64 bytes, r || s). Encoded as hex with 0x prefix.
    pub signature: String,
    /// 64-byte uncompressed public key (without 0x04 prefix). Encoded as hex with 0x prefix.
    pub signer: String,
}

/// Compress payload using `zstd`
///
/// Uses compression level 3 (default) for a good balance of speed and compression ratio.
fn compress_payload(data: &[u8]) -> Vec<u8> {
    let mut encoder = zstd::Encoder::new(Vec::new(), 3).expect("Failed to create zstd encoder");
    encoder
        .write_all(data)
        .expect("Failed to write data to encoder");
    encoder.finish().expect("Failed to finish compression")
}

/// The message inbox - assigns sequence numbers and signs messages using COSE
#[derive(Debug)]
pub struct Inbox {
    /// Current sequence counter (atomic for thread safety)
    sequence: AtomicU64,
    /// TEE key manager for signing
    key_manager: Arc<EvmKeyManager>,
}

impl Inbox {
    /// Create a new inbox with a shared key manager, starting from sequence 0
    pub const fn new(key_manager: Arc<EvmKeyManager>) -> Self {
        Self::with_start_sequence(key_manager, 0)
    }

    /// Create a new inbox with a shared key manager, starting from a specific sequence
    ///
    /// Use this when the key manager is shared with other components (e.g., batcher).
    pub const fn with_start_sequence(key_manager: Arc<EvmKeyManager>, start_sequence: u64) -> Self {
        Self {
            sequence: AtomicU64::new(start_sequence),
            key_manager,
        }
    }

    /// Get the signer's address
    pub fn signer_address(&self) -> Address {
        self.key_manager.address()
    }

    /// Get a reference to the key manager (for sharing with batcher)
    pub fn key_manager(&self) -> Arc<EvmKeyManager> {
        Arc::clone(&self.key_manager)
    }

    /// Get the current sequence number (next to be assigned)
    pub fn current_sequence(&self) -> u64 {
        self.sequence.load(Ordering::SeqCst)
    }

    /// Sequence and sign a message using COSE
    ///
    /// This atomically:
    /// 1. Assigns the next sequence number
    /// 2. Records the current timestamp
    /// 3. Compresses the payload with `zstd`
    /// 4. Creates a COSE `Sign1` message with 64-byte signature
    /// 5. Returns the signed message and a receipt
    pub fn sequence_message(
        &self,
        message_type: CborMessageType,
        payload: Vec<u8>,
    ) -> Result<(CborSignedMessage, SequenceReceipt), InboxError> {
        // Atomically get and increment sequence
        let sequence = self.sequence.fetch_add(1, Ordering::SeqCst);

        // Get current timestamp
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards")
            .as_secs();

        // Compress the payload
        let compressed_payload = compress_payload(&payload);

        // Hash the compressed payload (for receipt)
        let message_hash = keccak256(&compressed_payload);

        // Get signer's public key as `VerifyingKey`
        let pubkey_bytes = self.key_manager.public_key();
        let verifying_key = verifying_key_from_bytes(&pubkey_bytes)
            .map_err(|e| InboxError::InvalidPublicKey(e.to_string()))?;

        // Create COSE signed message
        let cbor_message = CborSignedMessage::new(
            sequence,
            timestamp,
            message_type,
            compressed_payload,
            &verifying_key,
            |data| self.sign_cose(data),
        )?;

        // Extract signature from COSE message for receipt
        let parsed = cbor_message.parse_without_verify()?;
        let signature_hex = format!("0x{}", hex::encode(parsed.signature));
        let signer_hex = format!("0x{}", hex::encode(pubkey_bytes));
        let message_hash_hex = format!("0x{}", hex::encode(message_hash));

        let receipt = SequenceReceipt {
            sequence,
            timestamp,
            message_hash: message_hash_hex,
            signature: signature_hex,
            signer: signer_hex,
        };

        Ok((cbor_message, receipt))
    }

    /// Sign data for COSE (returns ECDSA Signature)
    fn sign_cose(&self, data: &[u8]) -> Result<Signature, CborError> {
        let hash = keccak256(data);
        let alloy_sig = self
            .key_manager
            .sign_raw_sync(&hash.0)
            .map_err(|e| CborError::Signing(e.to_string()))?;

        let mut bytes = [0u8; 64];
        bytes[..32].copy_from_slice(&alloy_sig.r().to_be_bytes::<32>());
        bytes[32..].copy_from_slice(&alloy_sig.s().to_be_bytes::<32>());
        signature_from_bytes(&bytes)
    }
}

/// Verify that a receipt's signer matches the expected public key
///
/// The receipt contains a 64-byte uncompressed public key (hex with 0x prefix).
pub fn verify_receipt_signer(receipt: &SequenceReceipt, expected_pubkey: &[u8; 64]) -> bool {
    let receipt_pubkey_hex = receipt.signer.strip_prefix("0x").unwrap_or(&receipt.signer);
    let Ok(receipt_pubkey_bytes) = hex::decode(receipt_pubkey_hex) else {
        return false;
    };

    receipt_pubkey_bytes.as_slice() == expected_pubkey
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_key_manager() -> Arc<EvmKeyManager> {
        Arc::new(EvmKeyManager::generate())
    }

    #[test]
    fn test_inbox_creation() {
        let inbox = Inbox::new(test_key_manager());
        assert_eq!(inbox.current_sequence(), 0);
    }

    #[test]
    fn test_inbox_with_start_sequence() {
        let inbox = Inbox::with_start_sequence(test_key_manager(), 100);
        assert_eq!(inbox.current_sequence(), 100);
    }

    #[test]
    fn test_sequence_message() {
        let key_manager = test_key_manager();
        let pubkey = key_manager.public_key();
        let inbox = Inbox::new(key_manager);
        let payload = b"test payload".to_vec();

        let (cbor_msg, receipt) = inbox
            .sequence_message(CborMessageType::Changeset, payload)
            .unwrap();

        assert_eq!(receipt.sequence, 0);
        // COSE message should have non-zero size
        assert!(cbor_msg.size() > 0);
        // Receipt should have 64-byte signature (128 hex chars + 0x prefix)
        assert!(receipt.signature.starts_with("0x"));
        assert_eq!(receipt.signature.len(), 130); // 0x + 128 hex chars
                                                  // Receipt should have 64-byte public key (128 hex chars + 0x prefix)
        assert!(receipt.signer.starts_with("0x"));
        assert_eq!(receipt.signer.len(), 130); // 0x + 128 hex chars
                                               // Verify signer matches
        assert!(verify_receipt_signer(&receipt, &pubkey));
    }

    #[test]
    fn test_sequence_monotonic() {
        let inbox = Inbox::new(test_key_manager());

        let (_, receipt1) = inbox
            .sequence_message(CborMessageType::Changeset, b"msg1".to_vec())
            .unwrap();

        let (_, receipt2) = inbox
            .sequence_message(CborMessageType::Changeset, b"msg2".to_vec())
            .unwrap();

        let (_, receipt3) = inbox
            .sequence_message(CborMessageType::Withdrawal, b"msg3".to_vec())
            .unwrap();

        assert_eq!(receipt1.sequence, 0);
        assert_eq!(receipt2.sequence, 1);
        assert_eq!(receipt3.sequence, 2);
        assert_eq!(inbox.current_sequence(), 3);
    }

    #[test]
    fn test_receipt_signature_is_valid_cose() {
        let key_manager = test_key_manager();
        let pubkey_bytes = key_manager.public_key();
        let inbox = Inbox::new(key_manager);

        let (cbor_msg, receipt) = inbox
            .sequence_message(CborMessageType::Changeset, b"test".to_vec())
            .unwrap();

        // The COSE message should be verifiable
        let verifying_key = verifying_key_from_bytes(&pubkey_bytes).unwrap();
        let parsed = cbor_msg.verify_and_parse(&verifying_key).unwrap();

        assert_eq!(parsed.sequence, receipt.sequence);
        assert_eq!(parsed.timestamp, receipt.timestamp);

        // Signature in receipt should match the COSE signature
        let receipt_sig_hex = receipt.signature.strip_prefix("0x").unwrap();
        let receipt_sig = hex::decode(receipt_sig_hex).unwrap();
        assert_eq!(receipt_sig.as_slice(), parsed.signature.as_slice());
    }

    #[test]
    fn test_verify_receipt_signer() {
        let key_manager = test_key_manager();
        let expected_pubkey = key_manager.public_key();
        let inbox = Inbox::new(key_manager);

        let (_, receipt) = inbox
            .sequence_message(CborMessageType::Changeset, b"test".to_vec())
            .unwrap();

        assert!(verify_receipt_signer(&receipt, &expected_pubkey));

        // Wrong pubkey should fail
        let wrong_pubkey = [0x42u8; 64];
        assert!(!verify_receipt_signer(&receipt, &wrong_pubkey));
    }
}
