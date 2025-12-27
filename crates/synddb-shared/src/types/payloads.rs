//! Payload types for sequencer messages
//!
//! These types represent the payloads sent from clients to the sequencer.
//! The primary wire format is CBOR, but JSON with base64-encoded binary fields
//! is also supported for HTTP API compatibility.

use base64::Engine;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Custom serialization for binary data that works with both JSON (base64) and CBOR (raw bytes)
mod bytes_serde {
    use super::*;

    pub(super) fn serialize<S>(data: &[u8], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if serializer.is_human_readable() {
            // JSON: encode as base64 string
            let encoded = base64::engine::general_purpose::STANDARD.encode(data);
            serializer.serialize_str(&encoded)
        } else {
            // CBOR: serialize as raw bytes
            serializer.serialize_bytes(data)
        }
    }

    pub(super) fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        use serde::de::{Error, Visitor};

        struct BytesVisitor;

        impl<'de> Visitor<'de> for BytesVisitor {
            type Value = Vec<u8>;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a base64 string or byte array")
            }

            fn visit_str<E: Error>(self, v: &str) -> Result<Self::Value, E> {
                base64::engine::general_purpose::STANDARD
                    .decode(v)
                    .map_err(E::custom)
            }

            fn visit_bytes<E: Error>(self, v: &[u8]) -> Result<Self::Value, E> {
                Ok(v.to_vec())
            }

            fn visit_byte_buf<E: Error>(self, v: Vec<u8>) -> Result<Self::Value, E> {
                Ok(v)
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::SeqAccess<'de>,
            {
                let mut bytes = Vec::new();
                while let Some(byte) = seq.next_element::<u8>()? {
                    bytes.push(byte);
                }
                Ok(bytes)
            }
        }

        deserializer.deserialize_any(BytesVisitor)
    }
}

/// Changeset data from `synddb-client`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangesetData {
    /// Raw changeset bytes (base64-encoded in JSON, raw bytes in CBOR)
    #[serde(with = "bytes_serde")]
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
    /// Optional calldata (base64-encoded in JSON, raw bytes in CBOR)
    #[serde(default, with = "bytes_serde")]
    pub data: Vec<u8>,
}

/// Snapshot data from `synddb-client`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotData {
    /// Complete `SQLite` database file bytes (base64-encoded in JSON, raw bytes in CBOR)
    #[serde(with = "bytes_serde")]
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

// ============================================================================
// HTTP response types
// ============================================================================

/// Batch list item returned by `/storage/batches` endpoint
///
/// This is a minimal representation of batch metadata for listing purposes.
/// Use [`crate::types::batch::BatchInfo`] for full batch metadata including path and hash.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchListItem {
    /// First sequence number in this batch (inclusive)
    pub start_sequence: u64,
    /// Last sequence number in this batch (inclusive)
    pub end_sequence: u64,
}

// ============================================================================
// CBOR serialization helpers
// ============================================================================

impl ChangesetBatchRequest {
    /// Serialize to CBOR bytes
    pub fn to_cbor(&self) -> Result<Vec<u8>, ciborium::ser::Error<std::io::Error>> {
        let mut buf = Vec::new();
        ciborium::into_writer(self, &mut buf)?;
        Ok(buf)
    }

    /// Deserialize from CBOR bytes
    pub fn from_cbor(bytes: &[u8]) -> Result<Self, ciborium::de::Error<std::io::Error>> {
        ciborium::from_reader(bytes)
    }
}

impl WithdrawalRequest {
    /// Serialize to CBOR bytes
    pub fn to_cbor(&self) -> Result<Vec<u8>, ciborium::ser::Error<std::io::Error>> {
        let mut buf = Vec::new();
        ciborium::into_writer(self, &mut buf)?;
        Ok(buf)
    }

    /// Deserialize from CBOR bytes
    pub fn from_cbor(bytes: &[u8]) -> Result<Self, ciborium::de::Error<std::io::Error>> {
        ciborium::from_reader(bytes)
    }
}

impl SnapshotRequest {
    /// Serialize to CBOR bytes
    pub fn to_cbor(&self) -> Result<Vec<u8>, ciborium::ser::Error<std::io::Error>> {
        let mut buf = Vec::new();
        ciborium::into_writer(self, &mut buf)?;
        Ok(buf)
    }

    /// Deserialize from CBOR bytes
    pub fn from_cbor(bytes: &[u8]) -> Result<Self, ciborium::de::Error<std::io::Error>> {
        ciborium::from_reader(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_changeset_data_cbor_roundtrip() {
        let data = ChangesetData {
            data: b"test data".to_vec(),
            sequence: 42,
            timestamp: 1700000000,
        };

        let batch = ChangesetBatchRequest {
            batch_id: "batch-1".to_string(),
            changesets: vec![data],
            attestation_token: None,
        };

        let cbor = batch.to_cbor().unwrap();
        let decoded = ChangesetBatchRequest::from_cbor(&cbor).unwrap();

        assert_eq!(decoded.batch_id, "batch-1");
        assert_eq!(decoded.changesets.len(), 1);
        assert_eq!(decoded.changesets[0].data, b"test data");
        assert_eq!(decoded.changesets[0].sequence, 42);
    }

    #[test]
    fn test_withdrawal_request_cbor_roundtrip() {
        let request = WithdrawalRequest {
            request_id: "w1".to_string(),
            recipient: "0x742d35Cc6634C0532925a3b844Bc454e4438f44e".to_string(),
            amount: "1000000000000000000".to_string(),
            data: vec![],
        };

        let cbor = request.to_cbor().unwrap();
        let decoded = WithdrawalRequest::from_cbor(&cbor).unwrap();

        assert_eq!(decoded.request_id, "w1");
        assert_eq!(decoded.recipient, request.recipient);
        assert!(decoded.data.is_empty());
    }

    #[test]
    fn test_snapshot_request_cbor_roundtrip() {
        let request = SnapshotRequest {
            snapshot: SnapshotData {
                data: b"SQLite format 3\x00".to_vec(),
                timestamp: 1700000000,
                sequence: 100,
            },
            message_id: "snap-1".to_string(),
            attestation_token: Some("token123".to_string()),
        };

        let cbor = request.to_cbor().unwrap();
        let decoded = SnapshotRequest::from_cbor(&cbor).unwrap();

        assert_eq!(decoded.message_id, "snap-1");
        assert_eq!(decoded.snapshot.sequence, 100);
        assert_eq!(decoded.attestation_token, Some("token123".to_string()));
    }
}
