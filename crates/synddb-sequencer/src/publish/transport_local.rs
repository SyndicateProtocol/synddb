//! Local transport for CBOR batches with configurable storage backend
//!
//! Supports both in-memory storage (for testing) and `SQLite` file persistence
//! (for local development without external dependencies like GCS).
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
use rusqlite::{Connection, ErrorCode};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, RwLock},
};
use synddb_shared::types::{cbor::batch::CborBatch, message::SignedMessage};
use tracing::{debug, info};

// ============================================================================
// Configuration
// ============================================================================

/// Storage backend for `LocalTransport`
#[derive(Debug, Clone)]
pub enum LocalStorageBackend {
    /// In-memory storage (lost on restart)
    InMemory,
    /// `SQLite` file storage (persistent)
    Sqlite { path: String },
}

/// Configuration for `LocalTransport`
#[derive(Debug, Clone)]
pub struct LocalTransportConfig {
    /// Storage backend to use
    pub backend: LocalStorageBackend,
}

impl LocalTransportConfig {
    /// Create config for in-memory storage
    pub const fn in_memory() -> Self {
        Self {
            backend: LocalStorageBackend::InMemory,
        }
    }

    /// Create config for `SQLite` file storage
    pub fn file(path: impl Into<String>) -> Self {
        Self {
            backend: LocalStorageBackend::Sqlite { path: path.into() },
        }
    }
}

impl Default for LocalTransportConfig {
    fn default() -> Self {
        Self::in_memory()
    }
}

// ============================================================================
// Storage Internals
// ============================================================================

/// Internal storage implementation
enum StorageInner {
    /// In-memory storage using `BTreeMap`
    Memory(RwLock<MemoryState>),
    /// `SQLite` file storage
    Sqlite(Mutex<Connection>),
}

/// In-memory storage state
#[derive(Default)]
struct MemoryState {
    /// Batches indexed by `start_sequence`
    batches: BTreeMap<u64, StoredBatch>,
}

/// Stored batch data (used by in-memory backend)
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

// ============================================================================
// LocalTransport
// ============================================================================

/// Local transport for CBOR batches
///
/// Thread-safe storage that can be shared with HTTP handlers.
/// Supports both in-memory and `SQLite` file backends.
pub struct LocalTransport {
    storage: Arc<StorageInner>,
}

impl Clone for LocalTransport {
    fn clone(&self) -> Self {
        Self {
            storage: Arc::clone(&self.storage),
        }
    }
}

impl std::fmt::Debug for LocalTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let batch_count = match &*self.storage {
            StorageInner::Memory(state) => state.read().unwrap().batches.len(),
            StorageInner::Sqlite(conn) => {
                let conn = conn.lock().unwrap();
                conn.query_row("SELECT COUNT(*) FROM batches", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap_or(0) as usize
            }
        };
        f.debug_struct("LocalTransport")
            .field("batch_count", &batch_count)
            .finish()
    }
}

impl Default for LocalTransport {
    fn default() -> Self {
        Self::new(LocalTransportConfig::default())
    }
}

/// SQL schema for `SQLite` backend
const SQLITE_SCHEMA: &str = r#"
    CREATE TABLE IF NOT EXISTS batches (
        start_sequence INTEGER PRIMARY KEY,
        end_sequence INTEGER NOT NULL,
        compressed_data BLOB NOT NULL,
        uncompressed_size INTEGER NOT NULL,
        content_hash BLOB NOT NULL
    );
    CREATE INDEX IF NOT EXISTS idx_end_sequence ON batches(end_sequence);
"#;

impl LocalTransport {
    /// Create a new local transport with the given configuration
    pub fn new(config: LocalTransportConfig) -> Self {
        let storage = match config.backend {
            LocalStorageBackend::InMemory => {
                info!("Local CBOR transport initialized (in-memory)");
                StorageInner::Memory(RwLock::new(MemoryState::default()))
            }
            LocalStorageBackend::Sqlite { path } => {
                let conn = if path == ":memory:" {
                    Connection::open_in_memory().expect("Failed to open in-memory SQLite")
                } else {
                    Connection::open(&path)
                        .unwrap_or_else(|e| panic!("Failed to open SQLite at {path}: {e}"))
                };
                conn.execute_batch(SQLITE_SCHEMA)
                    .expect("Failed to create SQLite schema");
                info!(path = %path, "Local CBOR transport initialized (SQLite)");
                StorageInner::Sqlite(Mutex::new(conn))
            }
        };
        Self {
            storage: Arc::new(storage),
        }
    }

    /// Create a new in-memory transport (convenience method)
    pub fn in_memory() -> Self {
        Self::new(LocalTransportConfig::in_memory())
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
        self.get_batch_compressed(start_sequence)
            .and_then(|data| CborBatch::from_cbor_zstd(&data).ok())
    }

    /// Get compressed batch data by start sequence
    pub fn get_batch_compressed(&self, start_sequence: u64) -> Option<Vec<u8>> {
        match &*self.storage {
            StorageInner::Memory(state) => {
                let state = state.read().unwrap();
                state
                    .batches
                    .get(&start_sequence)
                    .map(|stored| stored.compressed_data.clone())
            }
            StorageInner::Sqlite(conn) => {
                let conn = conn.lock().unwrap();
                conn.query_row(
                    "SELECT compressed_data FROM batches WHERE start_sequence = ?1",
                    [start_sequence],
                    |row| row.get::<_, Vec<u8>>(0),
                )
                .ok()
            }
        }
    }

    /// List all batch metadata
    pub fn list_batch_info(&self) -> Vec<BatchInfo> {
        match &*self.storage {
            StorageInner::Memory(state) => {
                let state = state.read().unwrap();
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
            StorageInner::Sqlite(conn) => {
                let conn = conn.lock().unwrap();
                let mut stmt = conn
                    .prepare(
                        "SELECT start_sequence, end_sequence, content_hash FROM batches ORDER BY start_sequence",
                    )
                    .unwrap();
                stmt.query_map([], |row| {
                    let start_sequence: u64 = row.get(0)?;
                    let end_sequence: u64 = row.get(1)?;
                    let content_hash_blob: Vec<u8> = row.get(2)?;
                    let mut content_hash = [0u8; 32];
                    content_hash.copy_from_slice(&content_hash_blob);
                    Ok(BatchInfo {
                        start_sequence,
                        end_sequence,
                        reference: format!("local://{start_sequence}"),
                        content_hash,
                    })
                })
                .unwrap()
                .filter_map(Result::ok)
                .collect()
            }
        }
    }

    /// Get the latest sequence number
    pub fn latest_sequence(&self) -> Option<u64> {
        match &*self.storage {
            StorageInner::Memory(state) => {
                let state = state.read().unwrap();
                state.batches.values().map(|b| b.end_sequence).max()
            }
            StorageInner::Sqlite(conn) => {
                let conn = conn.lock().unwrap();
                conn.query_row("SELECT MAX(end_sequence) FROM batches", [], |row| {
                    row.get::<_, Option<u64>>(0)
                })
                .ok()
                .flatten()
            }
        }
    }

    /// Get a message by sequence number
    ///
    /// Searches all batches to find the message, converts to JSON `SignedMessage`.
    /// Note: This doesn't re-verify signatures since they were verified at publish time.
    pub fn get_message(&self, sequence: u64) -> Option<SignedMessage> {
        // Find the batch containing this sequence
        let compressed_data = match &*self.storage {
            StorageInner::Memory(state) => {
                let state = state.read().unwrap();
                state
                    .batches
                    .values()
                    .find(|stored| {
                        sequence >= stored.start_sequence && sequence <= stored.end_sequence
                    })
                    .map(|stored| stored.compressed_data.clone())
            }
            StorageInner::Sqlite(conn) => {
                let conn = conn.lock().unwrap();
                conn.query_row(
                    "SELECT compressed_data FROM batches WHERE start_sequence <= ?1 AND end_sequence >= ?1",
                    [sequence],
                    |row| row.get::<_, Vec<u8>>(0),
                )
                .ok()
            }
        };

        let compressed_data = compressed_data?;
        let batch = CborBatch::from_cbor_zstd(&compressed_data).ok()?;

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

        // Store in backend
        match &*self.storage {
            StorageInner::Memory(state) => {
                let stored = StoredBatch {
                    compressed_data: compressed,
                    uncompressed_size: uncompressed_bytes,
                    content_hash: batch.content_hash,
                    start_sequence: batch.start_sequence,
                    end_sequence: batch.end_sequence,
                };
                let mut state = state.write().unwrap();
                state.batches.insert(batch.start_sequence, stored);
            }
            StorageInner::Sqlite(conn) => {
                let conn = conn.lock().unwrap();
                let result = conn.execute(
                    "INSERT INTO batches (start_sequence, end_sequence, compressed_data, uncompressed_size, content_hash) VALUES (?1, ?2, ?3, ?4, ?5)",
                    rusqlite::params![
                        batch.start_sequence,
                        batch.end_sequence,
                        &compressed,
                        uncompressed_bytes,
                        &batch.content_hash[..],
                    ],
                );
                // Handle idempotent writes - duplicate is OK
                if let Err(rusqlite::Error::SqliteFailure(err, _)) = &result {
                    if err.code != ErrorCode::ConstraintViolation {
                        result.map_err(|e| {
                            TransportError::Storage(format!("SQLite insert failed: {e}"))
                        })?;
                    }
                    // Constraint violation = duplicate, which is fine (idempotent)
                }
            }
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
        let compressed_data = self.get_batch_compressed(start_sequence);

        let Some(data) = compressed_data else {
            debug!(
                start_sequence = start_sequence,
                "Batch not found in local storage"
            );
            return Ok(None);
        };

        let batch = CborBatch::from_cbor_zstd(&data)?;

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
        let transport = LocalTransport::in_memory();

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
        let transport = LocalTransport::in_memory();

        transport.publish(&create_test_batch(1, 5)).await.unwrap();
        transport.publish(&create_test_batch(6, 10)).await.unwrap();

        let batches = transport.list_batches().await.unwrap();
        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0].start_sequence, 1);
        assert_eq!(batches[1].start_sequence, 6);
    }

    #[tokio::test]
    async fn test_local_transport_latest_sequence() {
        let transport = LocalTransport::in_memory();

        assert_eq!(transport.get_latest_sequence().await.unwrap(), None);

        transport.publish(&create_test_batch(1, 5)).await.unwrap();
        assert_eq!(transport.get_latest_sequence().await.unwrap(), Some(5));

        transport.publish(&create_test_batch(6, 10)).await.unwrap();
        assert_eq!(transport.get_latest_sequence().await.unwrap(), Some(10));
    }

    #[tokio::test]
    async fn test_local_transport_get_message() {
        let transport = LocalTransport::in_memory();

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
        let transport = LocalTransport::in_memory();

        let result = transport.fetch(999).await.unwrap();
        assert!(result.is_none());
    }

    // ========================================================================
    // SQLite backend tests
    // ========================================================================

    /// Create a `SQLite` transport using :memory: for testing
    fn sqlite_transport() -> LocalTransport {
        LocalTransport::new(LocalTransportConfig::file(":memory:"))
    }

    #[tokio::test]
    async fn test_sqlite_publish_and_fetch() {
        let transport = sqlite_transport();

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
    async fn test_sqlite_list_batches() {
        let transport = sqlite_transport();

        transport.publish(&create_test_batch(1, 5)).await.unwrap();
        transport.publish(&create_test_batch(6, 10)).await.unwrap();

        let batches = transport.list_batches().await.unwrap();
        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0].start_sequence, 1);
        assert_eq!(batches[1].start_sequence, 6);
    }

    #[tokio::test]
    async fn test_sqlite_latest_sequence() {
        let transport = sqlite_transport();

        assert_eq!(transport.get_latest_sequence().await.unwrap(), None);

        transport.publish(&create_test_batch(1, 5)).await.unwrap();
        assert_eq!(transport.get_latest_sequence().await.unwrap(), Some(5));

        transport.publish(&create_test_batch(6, 10)).await.unwrap();
        assert_eq!(transport.get_latest_sequence().await.unwrap(), Some(10));
    }

    #[tokio::test]
    async fn test_sqlite_get_message() {
        let transport = sqlite_transport();

        transport.publish(&create_test_batch(1, 5)).await.unwrap();

        // Get message from batch
        let msg = transport.get_message(3).unwrap();
        assert_eq!(msg.sequence, 3);
        assert!(!msg.cose_protected_header.is_empty());

        // Non-existent message
        assert!(transport.get_message(100).is_none());
    }

    #[tokio::test]
    async fn test_sqlite_idempotent_publish() {
        let transport = sqlite_transport();

        let batch = create_test_batch(1, 5);

        // First publish should succeed
        let metadata1 = transport.publish(&batch).await.unwrap();

        // Second publish of same batch should also succeed (idempotent)
        let metadata2 = transport.publish(&batch).await.unwrap();

        // Both should return same reference
        assert_eq!(metadata1.reference, metadata2.reference);

        // Should still only have one batch
        let batches = transport.list_batches().await.unwrap();
        assert_eq!(batches.len(), 1);
    }

    #[tokio::test]
    async fn test_sqlite_persistence_across_restart() {
        use std::path::PathBuf;

        // Create a temp file path
        let temp_dir = std::env::temp_dir();
        let db_path: PathBuf = temp_dir.join(format!("test_sqlite_{}.db", std::process::id()));
        let db_path_str = db_path.to_string_lossy().to_string();

        // Clean up any existing file
        let _ = std::fs::remove_file(&db_path);

        // Create transport and publish batch
        {
            let transport = LocalTransport::new(LocalTransportConfig::file(&db_path_str));
            transport.publish(&create_test_batch(1, 5)).await.unwrap();
            transport.publish(&create_test_batch(6, 10)).await.unwrap();

            assert_eq!(transport.get_latest_sequence().await.unwrap(), Some(10));
        }
        // Transport dropped, connection closed

        // Reopen and verify data persisted
        {
            let transport = LocalTransport::new(LocalTransportConfig::file(&db_path_str));

            // Latest sequence should be recovered
            assert_eq!(transport.get_latest_sequence().await.unwrap(), Some(10));

            // Batches should be present
            let batches = transport.list_batches().await.unwrap();
            assert_eq!(batches.len(), 2);

            // Can fetch specific batch
            let fetched = transport.fetch(1).await.unwrap().unwrap();
            assert_eq!(fetched.start_sequence, 1);
            assert_eq!(fetched.end_sequence, 5);
        }

        // Clean up
        let _ = std::fs::remove_file(&db_path);
    }
}
