//! Local in-memory transport for CBOR batches
//!
//! Stores CBOR batches in memory with optional HTTP routes for serving:
//!
//! ```text
//! /storage/batches              - List all batches
//! /storage/batches/{start}      - Get batch by start sequence (CBOR+zstd format)
//! /storage/messages/{sequence}  - Get message by sequence
//! /storage/latest               - Get latest sequence number
//! ```
//!
//! This transport enables testing CBOR/COSE signatures with the HTTP fetcher,
//! providing the same binary format as GCS but without external dependencies.

use crate::publish::transport::{BatchInfo, PublishMetadata, TransportError, TransportPublisher};
use async_trait::async_trait;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use std::{
    collections::BTreeMap,
    sync::{Arc, RwLock},
};
use synddb_shared::types::{cbor::batch::CborBatch, message::SignedMessage};
use tracing::{debug, info};

/// Stored batch data
#[derive(Clone)]
struct StoredBatch {
    /// Raw CBOR+zstd compressed data
    compressed_data: Vec<u8>,
    /// Uncompressed size for stats
    uncompressed_size: usize,
    /// Content hash
    content_hash: [u8; 32],
    /// Start sequence
    start_sequence: u64,
    /// End sequence
    end_sequence: u64,
}

/// Shared state for `LocalTransport`
#[derive(Default)]
struct LocalTransportState {
    /// Batches indexed by `start_sequence`
    batches: BTreeMap<u64, StoredBatch>,
}

/// Local in-memory transport for CBOR batches
///
/// Thread-safe storage that can be shared with HTTP handlers.
#[derive(Clone)]
pub struct LocalTransport {
    state: Arc<RwLock<LocalTransportState>>,
}

impl std::fmt::Debug for LocalTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.state.read().unwrap();
        f.debug_struct("LocalTransport")
            .field("batch_count", &state.batches.len())
            .finish()
    }
}

impl Default for LocalTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl LocalTransport {
    /// Create a new local transport
    pub fn new() -> Self {
        info!("Local CBOR transport initialized");
        Self {
            state: Arc::new(RwLock::new(LocalTransportState::default())),
        }
    }

    /// Create an Axum router for serving batches over HTTP
    ///
    /// Mount these under `/storage/` on the sequencer router:
    /// ```ignore
    /// let app = Router::new()
    ///     .nest("/storage", transport.routes());
    /// ```
    pub fn routes(self: Arc<Self>) -> Router {
        Router::new()
            .route("/batches", get(list_batches))
            .route("/batches/{start}", get(get_batch_cbor))
            .route("/messages/{sequence}", get(get_message))
            .route("/latest", get(get_latest))
            .with_state(self)
    }

    /// Get a batch by start sequence (for internal use)
    pub fn get_batch(&self, start_sequence: u64) -> Option<CborBatch> {
        let state = self.state.read().unwrap();
        state
            .batches
            .get(&start_sequence)
            .and_then(|stored| CborBatch::from_cbor_zstd(&stored.compressed_data).ok())
    }

    /// Get compressed batch data by start sequence
    pub fn get_batch_compressed(&self, start_sequence: u64) -> Option<Vec<u8>> {
        let state = self.state.read().unwrap();
        state
            .batches
            .get(&start_sequence)
            .map(|stored| stored.compressed_data.clone())
    }

    /// List all batch metadata
    pub fn list_batch_info(&self) -> Vec<BatchInfo> {
        let state = self.state.read().unwrap();
        state
            .batches
            .values()
            .map(|stored| BatchInfo {
                start_sequence: stored.start_sequence,
                end_sequence: stored.end_sequence,
                reference: format!("local://{}", stored.start_sequence),
                content_hash: stored.content_hash,
            })
            .collect()
    }

    /// Get the latest sequence number
    pub fn latest_sequence(&self) -> Option<u64> {
        let state = self.state.read().unwrap();
        state.batches.values().map(|b| b.end_sequence).max()
    }

    /// Get a message by sequence number
    ///
    /// Searches all batches to find the message, converts to JSON `SignedMessage`.
    /// Note: This doesn't re-verify signatures since they were verified at publish time.
    pub fn get_message(&self, sequence: u64) -> Option<SignedMessage> {
        let state = self.state.read().unwrap();

        // Find the batch containing this sequence
        for stored in state.batches.values() {
            if sequence >= stored.start_sequence && sequence <= stored.end_sequence {
                if let Ok(batch) = CborBatch::from_cbor_zstd(&stored.compressed_data) {
                    // Find the message in this batch
                    for msg in &batch.messages {
                        if msg.sequence().ok() == Some(sequence) {
                            // Convert to SignedMessage (JSON format with cose_protected_header)
                            // Use unchecked since signatures were verified at publish time
                            if let Ok(signed_msg) = msg.to_signed_message_unchecked() {
                                return Some(signed_msg);
                            }
                        }
                    }
                }
                break;
            }
        }
        None
    }
}

#[async_trait]
impl TransportPublisher for LocalTransport {
    fn name(&self) -> &str {
        "local"
    }

    async fn publish(&self, batch: &CborBatch) -> Result<PublishMetadata, TransportError> {
        debug!(
            start_sequence = batch.start_sequence,
            end_sequence = batch.end_sequence,
            message_count = batch.messages.len(),
            "Serializing batch for local storage"
        );

        // Serialize to CBOR with zstd compression
        let uncompressed = batch.to_cbor()?;
        let compressed = batch.to_cbor_zstd()?;

        let uncompressed_bytes = uncompressed.len();
        let compressed_bytes = compressed.len();
        let compression_ratio = uncompressed_bytes as f64 / compressed_bytes as f64;

        let stored = StoredBatch {
            compressed_data: compressed,
            uncompressed_size: uncompressed_bytes,
            content_hash: batch.content_hash,
            start_sequence: batch.start_sequence,
            end_sequence: batch.end_sequence,
        };

        // Store in memory
        {
            let mut state = self.state.write().unwrap();
            state.batches.insert(batch.start_sequence, stored);
        }

        let reference = format!("local://{}", batch.start_sequence);

        info!(
            start_sequence = batch.start_sequence,
            end_sequence = batch.end_sequence,
            message_count = batch.messages.len(),
            compressed_bytes = compressed_bytes,
            compression_ratio = format!("{:.2}x", compression_ratio),
            reference = %reference,
            "Batch published to local storage"
        );

        Ok(PublishMetadata {
            reference,
            content_hash: batch.content_hash,
            compressed_bytes,
            uncompressed_bytes,
        })
    }

    async fn fetch(&self, start_sequence: u64) -> Result<Option<CborBatch>, TransportError> {
        let state = self.state.read().unwrap();

        let Some(stored) = state.batches.get(&start_sequence) else {
            debug!(
                start_sequence = start_sequence,
                "Batch not found in local storage"
            );
            return Ok(None);
        };

        let batch = CborBatch::from_cbor_zstd(&stored.compressed_data)?;

        info!(
            start_sequence = batch.start_sequence,
            end_sequence = batch.end_sequence,
            message_count = batch.messages.len(),
            content_hash = %batch.content_hash_hex(),
            "Batch fetched from local storage"
        );

        Ok(Some(batch))
    }

    async fn list_batches(&self) -> Result<Vec<BatchInfo>, TransportError> {
        Ok(self.list_batch_info())
    }

    async fn get_latest_sequence(&self) -> Result<Option<u64>, TransportError> {
        Ok(self.latest_sequence())
    }
}

// ============================================================================
// HTTP Handlers
// ============================================================================

/// Response for batch list endpoint
#[derive(serde::Serialize)]
struct BatchListItem {
    start_sequence: u64,
    end_sequence: u64,
}

/// Response for latest endpoint
#[derive(serde::Serialize)]
struct LatestResponse {
    sequence: Option<u64>,
}

/// List all batches
async fn list_batches(State(transport): State<Arc<LocalTransport>>) -> Json<Vec<BatchListItem>> {
    let batches = transport.list_batch_info();
    let items: Vec<BatchListItem> = batches
        .into_iter()
        .map(|b| BatchListItem {
            start_sequence: b.start_sequence,
            end_sequence: b.end_sequence,
        })
        .collect();
    Json(items)
}

/// Get batch as raw CBOR+zstd
async fn get_batch_cbor(
    State(transport): State<Arc<LocalTransport>>,
    Path(start): Path<u64>,
) -> Response {
    transport.get_batch_compressed(start).map_or_else(
        || StatusCode::NOT_FOUND.into_response(),
        |data| {
            (
                StatusCode::OK,
                [("content-type", "application/cbor+zstd")],
                data,
            )
                .into_response()
        },
    )
}

/// Get message by sequence (returns JSON `SignedMessage`)
async fn get_message(
    State(transport): State<Arc<LocalTransport>>,
    Path(sequence): Path<u64>,
) -> Response {
    transport.get_message(sequence).map_or_else(
        || StatusCode::NOT_FOUND.into_response(),
        |msg| Json(msg).into_response(),
    )
}

/// Get latest sequence number
async fn get_latest(State(transport): State<Arc<LocalTransport>>) -> Json<LatestResponse> {
    Json(LatestResponse {
        sequence: transport.latest_sequence(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use synddb_shared::types::cbor::message::{CborMessageType, CborSignedMessage};

    fn create_test_message(sequence: u64) -> CborSignedMessage {
        let payload = format!("test payload {}", sequence).into_bytes();
        let timestamp = 1700000000 + sequence;
        let pubkey = [0x42u8; 64]; // Mock 64-byte public key

        CborSignedMessage::new(
            sequence,
            timestamp,
            CborMessageType::Changeset,
            payload,
            pubkey,
            |_data| Ok([0u8; 64]), // Mock signature
        )
        .unwrap()
    }

    fn create_test_batch(start: u64, end: u64) -> CborBatch {
        let messages: Vec<CborSignedMessage> = (start..=end).map(create_test_message).collect();
        let created_at = 1700000000;
        let pubkey = [0x42u8; 64]; // Mock 64-byte public key

        CborBatch::new(messages, created_at, pubkey, |_data| Ok([0u8; 64])).unwrap()
    }

    #[tokio::test]
    async fn test_local_transport_publish_and_fetch() {
        let transport = LocalTransport::new();

        let batch = create_test_batch(1, 5);
        let metadata = transport.publish(&batch).await.unwrap();

        assert!(metadata.reference.starts_with("local://"));
        assert!(metadata.compressed_bytes > 0);

        // Fetch it back
        let fetched = transport.fetch(1).await.unwrap().unwrap();
        assert_eq!(fetched.start_sequence, 1);
        assert_eq!(fetched.end_sequence, 5);
        assert_eq!(fetched.messages.len(), 5);
    }

    #[tokio::test]
    async fn test_local_transport_list_batches() {
        let transport = LocalTransport::new();

        transport.publish(&create_test_batch(1, 5)).await.unwrap();
        transport.publish(&create_test_batch(6, 10)).await.unwrap();

        let batches = transport.list_batches().await.unwrap();
        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0].start_sequence, 1);
        assert_eq!(batches[1].start_sequence, 6);
    }

    #[tokio::test]
    async fn test_local_transport_latest_sequence() {
        let transport = LocalTransport::new();

        assert_eq!(transport.get_latest_sequence().await.unwrap(), None);

        transport.publish(&create_test_batch(1, 5)).await.unwrap();
        assert_eq!(transport.get_latest_sequence().await.unwrap(), Some(5));

        transport.publish(&create_test_batch(6, 10)).await.unwrap();
        assert_eq!(transport.get_latest_sequence().await.unwrap(), Some(10));
    }

    #[tokio::test]
    async fn test_local_transport_get_message() {
        let transport = LocalTransport::new();

        transport.publish(&create_test_batch(1, 5)).await.unwrap();

        // Get message from batch
        let msg = transport.get_message(3).unwrap();
        assert_eq!(msg.sequence, 3);
        // COSE message should have protected header
        assert!(!msg.cose_protected_header.is_empty());

        // Non-existent message
        assert!(transport.get_message(100).is_none());
    }

    #[tokio::test]
    async fn test_local_transport_fetch_not_found() {
        let transport = LocalTransport::new();

        let result = transport.fetch(999).await.unwrap();
        assert!(result.is_none());
    }
}
