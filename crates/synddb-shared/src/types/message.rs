//! Core message types for sequencer wire format

use serde::{Deserialize, Serialize};

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

/// A message that has been sequenced and signed by the sequencer
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedMessage {
    /// Monotonically increasing sequence number
    pub sequence: u64,
    /// Unix timestamp (seconds) when sequenced
    pub timestamp: u64,
    /// Type of message
    pub message_type: MessageType,
    /// Compressed message payload (`zstd`-compressed JSON)
    pub payload: Vec<u8>,
    /// Hash of the compressed payload: `keccak256(compressed_payload)`
    pub message_hash: String,
    /// Signature over `(sequence || timestamp || message_hash)`
    pub signature: String,
    /// Ethereum address of the signer
    pub signer: String,
}

/// A batch of signed messages for atomic publication
///
/// Batches combine multiple messages into a single atomic unit for publication
/// to data availability layers. This ensures that messages and state are always
/// published together, preventing partial publication failures.
///
/// # Storage Format
///
/// Batches are stored with filenames indicating their sequence range:
/// ```text
/// gs://{bucket}/{prefix}/batches/{start:012}_{end:012}.json
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
    /// The messages in this batch
    pub messages: Vec<SignedMessage>,
    /// Signature over the batch: `keccak256(start_sequence || end_sequence || messages_hash)`
    /// where `messages_hash = keccak256(canonical_json(messages))`
    pub batch_signature: String,
    /// Ethereum address of the batch signer (same as message signer)
    pub signer: String,
    /// Unix timestamp (seconds) when the batch was created
    pub created_at: u64,
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
            },
            SignedMessage {
                sequence: 2,
                timestamp: 1700000001,
                message_type: MessageType::Withdrawal,
                payload: vec![4, 5, 6],
                message_hash: "0xghi".to_string(),
                signature: "0xjkl".to_string(),
                signer: "0x123".to_string(),
            },
        ];

        let batch = SignedBatch {
            start_sequence: 1,
            end_sequence: 2,
            messages,
            batch_signature: "0xbatchsig".to_string(),
            signer: "0x123".to_string(),
            created_at: 1700000002,
        };

        let json = serde_json::to_string(&batch).unwrap();
        let decoded: SignedBatch = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded.start_sequence, 1);
        assert_eq!(decoded.end_sequence, 2);
        assert_eq!(decoded.messages.len(), 2);
        assert_eq!(decoded.messages[0].sequence, 1);
        assert_eq!(decoded.messages[1].sequence, 2);
    }
}
