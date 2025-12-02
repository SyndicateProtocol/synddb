//! Payload types for sequencer messages
//!
//! These types represent the JSON payloads that are zstd-compressed
//! and stored in `SignedMessage.payload`.

use serde::{Deserialize, Serialize};

use super::serde_helpers::base64_serde;

/// Changeset data from `synddb-client`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangesetData {
    /// Raw changeset bytes (base64 encoded in JSON)
    #[serde(with = "base64_serde")]
    pub data: Vec<u8>,
    /// Client-side sequence number
    pub sequence: u64,
    /// Client-side timestamp (Unix timestamp in seconds)
    pub timestamp: u64,
}

/// Changeset batch request from `synddb-client`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangesetBatchRequest {
    /// Batch identifier for tracking
    pub batch_id: String,
    /// List of changesets in this batch
    pub changesets: Vec<ChangesetData>,
    /// Optional TEE attestation token
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attestation_token: Option<String>,
}

/// Withdrawal request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WithdrawalRequest {
    /// Unique request identifier
    pub request_id: String,
    /// Recipient address (Ethereum format)
    pub recipient: String,
    /// Amount to withdraw (as string to handle large numbers)
    pub amount: String,
    /// Optional calldata
    #[serde(default, with = "base64_serde")]
    pub data: Vec<u8>,
}

/// Snapshot data from `synddb-client`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotData {
    /// Complete `SQLite` database file bytes (base64 encoded in JSON)
    #[serde(with = "base64_serde")]
    pub data: Vec<u8>,
    /// Client-side timestamp (Unix timestamp in seconds)
    pub timestamp: u64,
    /// Client-side sequence number (which changesets are included)
    pub sequence: u64,
}

/// Snapshot request from `synddb-client`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotRequest {
    /// Snapshot data
    pub snapshot: SnapshotData,
    /// Message identifier for tracking
    pub message_id: String,
    /// Optional TEE attestation token
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attestation_token: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_changeset_data_serialization() {
        let data = ChangesetData {
            data: b"test data".to_vec(),
            sequence: 42,
            timestamp: 1700000000,
        };

        let json = serde_json::to_string(&data).unwrap();
        // Verify base64 encoding: "test data" -> "dGVzdCBkYXRh"
        assert!(json.contains("dGVzdCBkYXRh"));

        let decoded: ChangesetData = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.data, b"test data");
        assert_eq!(decoded.sequence, 42);
    }

    #[test]
    fn test_changeset_batch_request() {
        let batch = ChangesetBatchRequest {
            batch_id: "batch-1".to_string(),
            changesets: vec![ChangesetData {
                data: b"cs1".to_vec(),
                sequence: 0,
                timestamp: 1700000000,
            }],
            attestation_token: None,
        };

        let json = serde_json::to_string(&batch).unwrap();
        // attestation_token should be omitted when None
        assert!(!json.contains("attestation_token"));

        let decoded: ChangesetBatchRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.batch_id, "batch-1");
        assert_eq!(decoded.changesets.len(), 1);
    }

    #[test]
    fn test_withdrawal_request_with_empty_data() {
        // Test deserialization with empty data field
        let json = r#"{
            "request_id": "w1",
            "recipient": "0x742d35Cc6634C0532925a3b844Bc454e4438f44e",
            "amount": "1000000000000000000",
            "data": ""
        }"#;

        let decoded: WithdrawalRequest = serde_json::from_str(json).unwrap();
        assert_eq!(decoded.request_id, "w1");
        assert!(decoded.data.is_empty());
    }

    #[test]
    fn test_snapshot_request() {
        let request = SnapshotRequest {
            snapshot: SnapshotData {
                data: b"SQLite format 3\x00".to_vec(),
                timestamp: 1700000000,
                sequence: 100,
            },
            message_id: "snap-1".to_string(),
            attestation_token: Some("token123".to_string()),
        };

        let json = serde_json::to_string(&request).unwrap();
        let decoded: SnapshotRequest = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded.message_id, "snap-1");
        assert_eq!(decoded.snapshot.sequence, 100);
        assert_eq!(decoded.attestation_token, Some("token123".to_string()));
    }
}
