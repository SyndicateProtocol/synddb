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
#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// Receipt returned to clients after successful sequencing
#[derive(Debug, Clone, Serialize, Deserialize)]
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
}
