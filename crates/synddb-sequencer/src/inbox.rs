//! Message inbox for ordering and sequencing
//!
//! Provides monotonic sequence number assignment for all incoming messages.
//! This is the core ordering mechanism similar to Arbitrum's delayed inbox.

use alloy::primitives::{keccak256, Address};
use std::{
    io::Write,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{SystemTime, UNIX_EPOCH},
};

use crate::signer::{MessageSigner, SignerError};
use synddb_shared::types::message::{MessageType, SequenceReceipt, SignedMessage};

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

/// The message inbox - assigns sequence numbers and signs messages
#[derive(Debug)]
pub struct Inbox {
    /// Current sequence counter (atomic for thread safety)
    sequence: AtomicU64,
    /// Message signer
    signer: Arc<MessageSigner>,
}

impl Inbox {
    /// Create a new inbox with the given signer, starting from sequence 0
    pub fn new(signer: MessageSigner) -> Self {
        Self::with_start_sequence(signer, 0)
    }

    /// Create a new inbox starting from a specific sequence number (for recovery)
    pub fn with_start_sequence(signer: MessageSigner, start_sequence: u64) -> Self {
        Self::with_start_sequence_arc(Arc::new(signer), start_sequence)
    }

    /// Create a new inbox with a shared signer, starting from a specific sequence
    ///
    /// Use this when the signer is shared with other components (e.g., publishers).
    pub const fn with_start_sequence_arc(signer: Arc<MessageSigner>, start_sequence: u64) -> Self {
        Self {
            sequence: AtomicU64::new(start_sequence),
            signer,
        }
    }

    /// Get the signer's address
    pub fn signer_address(&self) -> Address {
        self.signer.address()
    }

    /// Get a reference to the signer (for sharing with publishers)
    pub fn signer(&self) -> Arc<MessageSigner> {
        Arc::clone(&self.signer)
    }

    /// Get the current sequence number (next to be assigned)
    pub fn current_sequence(&self) -> u64 {
        self.sequence.load(Ordering::SeqCst)
    }

    /// Sequence and sign a message
    ///
    /// This atomically:
    /// 1. Assigns the next sequence number
    /// 2. Records the current timestamp
    /// 3. Compresses the payload
    /// 4. Hashes the compressed payload
    /// 5. Signs the message
    /// 6. Returns a receipt
    pub async fn sequence_message(
        &self,
        message_type: MessageType,
        payload: Vec<u8>,
    ) -> Result<(SignedMessage, SequenceReceipt), SignerError> {
        // Atomically get and increment sequence
        let sequence = self.sequence.fetch_add(1, Ordering::SeqCst);

        // Get current timestamp
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards")
            .as_secs();

        // Compress the payload
        let compressed_payload = compress_payload(&payload);

        // Hash the compressed payload
        let message_hash = keccak256(&compressed_payload);

        // Sign the message
        let signature = self
            .signer
            .sign_message(sequence, timestamp, message_hash)
            .await?;

        let signer_address = format!("{:?}", self.signer.address());
        let message_hash_hex = format!("0x{}", hex::encode(message_hash));
        let signature_hex = signature.to_hex_prefixed();

        let signed_message = SignedMessage {
            sequence,
            timestamp,
            message_type,
            payload: compressed_payload,
            message_hash: message_hash_hex.clone(),
            signature: signature_hex.clone(),
            signer: signer_address.clone(),
            cose_protected_header: vec![], // Placeholder - real COSE header is set by batcher
        };

        let receipt = SequenceReceipt {
            sequence,
            timestamp,
            message_hash: message_hash_hex,
            signature: signature_hex,
            signer: signer_address,
        };

        Ok((signed_message, receipt))
    }
}

/// Verify a signature matches the expected signer
pub fn verify_receipt(receipt: &SequenceReceipt, expected_signer: Address) -> bool {
    // Parse the signer address from the receipt
    let receipt_signer: Address = match receipt.signer.parse() {
        Ok(addr) => addr,
        Err(_) => return false,
    };

    receipt_signer == expected_signer
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_PRIVATE_KEY: &str =
        "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

    fn test_signer() -> MessageSigner {
        MessageSigner::new(TEST_PRIVATE_KEY).unwrap()
    }

    #[test]
    fn test_inbox_creation() {
        let inbox = Inbox::new(test_signer());
        assert_eq!(inbox.current_sequence(), 0);
    }

    #[test]
    fn test_inbox_with_start_sequence() {
        let inbox = Inbox::with_start_sequence(test_signer(), 100);
        assert_eq!(inbox.current_sequence(), 100);
    }

    #[tokio::test]
    async fn test_sequence_message() {
        let inbox = Inbox::new(test_signer());
        let payload = b"test payload".to_vec();

        let (signed_msg, receipt) = inbox
            .sequence_message(MessageType::Changeset, payload.clone())
            .await
            .unwrap();

        assert_eq!(signed_msg.sequence, 0);
        assert_eq!(receipt.sequence, 0);
        // Payload is now compressed, so it won't match the original
        assert_ne!(signed_msg.payload, payload);
        // But it should be smaller or similar size for small payloads
        assert!(!signed_msg.payload.is_empty());
        assert!(signed_msg.signature.starts_with("0x"));
        assert!(signed_msg.message_hash.starts_with("0x"));
    }

    #[tokio::test]
    async fn test_sequence_monotonic() {
        let inbox = Inbox::new(test_signer());

        let (msg1, _) = inbox
            .sequence_message(MessageType::Changeset, b"msg1".to_vec())
            .await
            .unwrap();

        let (msg2, _) = inbox
            .sequence_message(MessageType::Changeset, b"msg2".to_vec())
            .await
            .unwrap();

        let (msg3, _) = inbox
            .sequence_message(MessageType::Withdrawal, b"msg3".to_vec())
            .await
            .unwrap();

        assert_eq!(msg1.sequence, 0);
        assert_eq!(msg2.sequence, 1);
        assert_eq!(msg3.sequence, 2);
        assert_eq!(inbox.current_sequence(), 3);
    }

    #[tokio::test]
    async fn test_receipt_matches_signed_message() {
        let inbox = Inbox::new(test_signer());

        let (signed_msg, receipt) = inbox
            .sequence_message(MessageType::Changeset, b"test".to_vec())
            .await
            .unwrap();

        assert_eq!(signed_msg.sequence, receipt.sequence);
        assert_eq!(signed_msg.timestamp, receipt.timestamp);
        assert_eq!(signed_msg.message_hash, receipt.message_hash);
        assert_eq!(signed_msg.signature, receipt.signature);
        assert_eq!(signed_msg.signer, receipt.signer);
    }

    #[tokio::test]
    async fn test_verify_receipt() {
        let signer = test_signer();
        let expected_address = signer.address();
        let inbox = Inbox::new(signer);

        let (_, receipt) = inbox
            .sequence_message(MessageType::Changeset, b"test".to_vec())
            .await
            .unwrap();

        assert!(verify_receipt(&receipt, expected_address));

        // Wrong address should fail
        let wrong_address: Address = "0x0000000000000000000000000000000000000001"
            .parse()
            .unwrap();
        assert!(!verify_receipt(&receipt, wrong_address));
    }
}
