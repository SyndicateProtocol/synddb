//! Local `SQLite`-based publisher for self-hosted deployments
//!
//! This publisher stores signed batches in a local `SQLite` database, providing
//! a simple alternative to external DA layers like GCS for customers who don't
//! need distributed storage.
//!
//! # Storage Schema
//!
//! ```sql
//! CREATE TABLE batches (
//!     start_sequence INTEGER PRIMARY KEY,
//!     end_sequence INTEGER NOT NULL,
//!     data BLOB NOT NULL,
//!     created_at INTEGER NOT NULL
//! );
//! CREATE INDEX idx_end_sequence ON batches(end_sequence);
//! ```
//!
//! # HTTP API
//!
//! When mounted on the sequencer, exposes these endpoints under `/da/`:
//!
//! - `GET /da/batches/{start}` - Retrieve batch by start sequence
//! - `GET /da/messages/{sequence}` - Retrieve message by sequence
//! - `GET /da/latest` - Get latest published sequence number
//!
//! # Usage
//!
//! ```ignore
//! // In-memory (for tests)
//! let publisher = LocalPublisher::new(LocalConfig::in_memory(), signer)?;
//!
//! // File-backed (for production)
//! let publisher = LocalPublisher::new(LocalConfig::file("/data/local_da.db"), signer)?;
//!
//! // Mount HTTP routes
//! let router = sequencer_router.nest("/da", publisher.routes());
//! ```

use async_trait::async_trait;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use tracing::{error, info, warn};

use crate::publish::traits::{DAPublisher, PublishError, PublishResult};
use crate::signer::MessageSigner;
use synddb_shared::types::message::{SignedBatch, SignedMessage};

/// Configuration for the local publisher
#[derive(Debug, Clone)]
pub struct LocalConfig {
    /// Database path (":memory:" for in-memory)
    pub db_path: String,
}

impl LocalConfig {
    /// Create config for in-memory `SQLite` (ephemeral, for tests)
    pub fn in_memory() -> Self {
        Self {
            db_path: ":memory:".to_string(),
        }
    }

    /// Create config for file-backed `SQLite` (durable, for production)
    pub fn file(path: impl Into<String>) -> Self {
        Self {
            db_path: path.into(),
        }
    }
}

/// Local `SQLite`-based publisher
///
/// Stores signed batches in `SQLite` for retrieval by validators.
/// Suitable for self-hosted deployments that don't need external DA.
pub struct LocalPublisher {
    conn: Mutex<Connection>,
    config: LocalConfig,
    signer: Arc<MessageSigner>,
}

impl std::fmt::Debug for LocalPublisher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalPublisher")
            .field("config", &self.config)
            .field("signer", &format!("{:?}", self.signer.address()))
            .finish()
    }
}

impl LocalPublisher {
    /// Create a new local publisher
    pub fn new(config: LocalConfig, signer: Arc<MessageSigner>) -> Result<Self, PublishError> {
        let conn = Connection::open(&config.db_path)
            .map_err(|e| PublishError::Storage(format!("Failed to open database: {e}")))?;

        // Initialize schema
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS batches (start_sequence INTEGER PRIMARY KEY,
                end_sequence INTEGER NOT NULL,
                data BLOB NOT NULL,
                created_at INTEGER NOT NULL);
            CREATE INDEX IF NOT EXISTS idx_end_sequence ON batches(end_sequence);
            "#,
        )
        .map_err(|e| PublishError::Storage(format!("Failed to create schema: {e}")))?;

        let db_type = if config.db_path == ":memory:" {
            "in-memory"
        } else {
            "file"
        };
        info!(
            db_path = %config.db_path,
            db_type = db_type,
            signer = %format!("{:?}", signer.address()),
            "Local publisher initialized"
        );

        Ok(Self {
            conn: Mutex::new(conn),
            config,
            signer,
        })
    }

    /// Create an Arc-wrapped publisher (convenience for shared ownership)
    pub fn new_arc(
        config: LocalConfig,
        signer: Arc<MessageSigner>,
    ) -> Result<Arc<Self>, PublishError> {
        Ok(Arc::new(Self::new(config, signer)?))
    }

    /// Get HTTP routes for the DA fetch API
    ///
    /// Mount these under `/da/` on the sequencer router:
    /// ```ignore
    /// let router = sequencer_router.nest("/da", publisher.routes());
    /// ```
    pub fn routes(self: &Arc<Self>) -> Router {
        Router::new()
            .route("/batches/{start}", get(get_batch_handler))
            .route("/messages/{sequence}", get(get_message_handler))
            .route("/latest", get(get_latest_handler))
            .with_state(Arc::clone(self))
    }

    /// Find the batch containing a specific sequence number
    fn find_batch_containing(&self, sequence: u64) -> Result<Option<SignedBatch>, PublishError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT data FROM batches
                 WHERE start_sequence <= ?1 AND end_sequence >= ?1
                 LIMIT 1",
            )
            .map_err(|e| PublishError::Storage(format!("Query prepare failed: {e}")))?;

        let result: Option<Vec<u8>> = stmt
            .query_row([sequence], |row| row.get(0))
            .optional()
            .map_err(|e| PublishError::Storage(format!("Query failed: {e}")))?;

        match result {
            Some(data) => {
                let batch: SignedBatch = serde_json::from_slice(&data)
                    .map_err(|e| PublishError::Serialization(format!("Failed to parse batch: {e}")))?;
                Ok(Some(batch))
            }
            None => Ok(None),
        }
    }
}

#[async_trait]
impl DAPublisher for LocalPublisher {
    fn name(&self) -> &str {
        "local"
    }

    async fn publish(&self, message: &SignedMessage) -> PublishResult {
        // Wrap single message in a batch with proper batch signature
        let messages = vec![message.clone()];

        // Serialize messages for hashing
        let messages_json = match serde_json::to_vec(&messages) {
            Ok(json) => json,
            Err(e) => {
                error!(sequence = message.sequence, error = %e, "Failed to serialize messages");
                return PublishResult::failure("local", format!("Serialization error: {e}"));
            }
        };

        // Compute messages hash and sign the batch
        let messages_hash = MessageSigner::compute_messages_hash(&messages_json);
        let batch_signature = match self
            .signer
            .sign_batch(message.sequence, message.sequence, messages_hash)
            .await
        {
            Ok(sig) => sig.to_hex_prefixed(),
            Err(e) => {
                error!(sequence = message.sequence, error = %e, "Failed to sign batch");
                return PublishResult::failure("local", format!("Signing error: {e}"));
            }
        };

        let batch = SignedBatch {
            start_sequence: message.sequence,
            end_sequence: message.sequence,
            messages,
            batch_signature,
            signer: format!("{:?}", self.signer.address()),
            created_at: message.timestamp,
        };

        self.publish_batch(&batch).await
    }

    async fn publish_batch(&self, batch: &SignedBatch) -> PublishResult {
        // Verify batch signature before storing
        if let Err(e) = batch.verify_batch_signature() {
            error!(
                start = batch.start_sequence,
                end = batch.end_sequence,
                error = %e,
                "Batch signature verification failed"
            );
            return PublishResult::failure("local", format!("Signature verification failed: {e}"));
        }

        // Serialize batch
        let data = match serde_json::to_vec(batch) {
            Ok(d) => d,
            Err(e) => {
                error!(
                    start = batch.start_sequence,
                    end = batch.end_sequence,
                    error = %e,
                    "Failed to serialize batch"
                );
                return PublishResult::failure("local", format!("Serialization error: {e}"));
            }
        };

        // Store in SQLite
        let result = {
            let conn = self.conn.lock().unwrap();
            conn.execute(
                "INSERT OR REPLACE INTO batches (start_sequence, end_sequence, data, created_at)
                 VALUES (?1, ?2, ?3, ?4)",
                (
                    batch.start_sequence,
                    batch.end_sequence,
                    &data,
                    batch.created_at,
                ),
            )
        };

        match result {
            Ok(_) => {
                info!(
                    start = batch.start_sequence,
                    end = batch.end_sequence,
                    messages = batch.messages.len(),
                    "Batch published to local storage"
                );
                let reference = format!(
                    "local://batches/{}_{}",
                    batch.start_sequence,
                    batch.end_sequence
                );
                PublishResult::success("local", reference)
            }
            Err(e) => {
                error!(
                    start = batch.start_sequence,
                    end = batch.end_sequence,
                    error = %e,
                    "Failed to store batch"
                );
                PublishResult::failure("local", format!("Storage error: {e}"))
            }
        }
    }

    async fn get(&self, sequence: u64) -> Result<Option<SignedMessage>, PublishError> {
        let batch = self.find_batch_containing(sequence)?;
        Ok(batch.and_then(|b| b.messages.into_iter().find(|m| m.sequence == sequence)))
    }

    async fn get_batch(&self, start_sequence: u64) -> Result<Option<SignedBatch>, PublishError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT data FROM batches WHERE start_sequence = ?1")
            .map_err(|e| PublishError::Storage(format!("Query prepare failed: {e}")))?;

        let result: Option<Vec<u8>> = stmt
            .query_row([start_sequence], |row| row.get(0))
            .optional()
            .map_err(|e| PublishError::Storage(format!("Query failed: {e}")))?;

        match result {
            Some(data) => {
                let batch: SignedBatch = serde_json::from_slice(&data)
                    .map_err(|e| PublishError::Serialization(format!("Failed to parse batch: {e}")))?;
                Ok(Some(batch))
            }
            None => Ok(None),
        }
    }

    async fn get_latest_sequence(&self) -> Result<Option<u64>, PublishError> {
        let conn = self.conn.lock().unwrap();
        let result: Option<u64> = conn
            .query_row("SELECT MAX(end_sequence) FROM batches", [], |row| row.get(0))
            .optional()
            .map_err(|e| PublishError::Storage(format!("Query failed: {e}")))?
            .flatten();

        Ok(result)
    }

    async fn save_state(&self, _sequence: u64) -> Result<(), PublishError> {
        // State is implicit in batch records
        Ok(())
    }

    async fn load_state(&self) -> Result<Option<u64>, PublishError> {
        let seq = self.get_latest_sequence().await?;
        if let Some(s) = seq {
            info!(sequence = s, "Loaded state from local storage");
        } else {
            warn!("No existing batches in local storage, starting fresh");
        }
        Ok(seq)
    }
}

// ============================================================================
// HTTP Handlers for DA Fetch API
// ============================================================================

/// Response for latest sequence endpoint
#[derive(Debug, Serialize, Deserialize)]
pub struct LatestSequenceResponse {
    pub sequence: Option<u64>,
}

/// Get batch by start sequence
async fn get_batch_handler(
    State(publisher): State<Arc<LocalPublisher>>,
    Path(start): Path<u64>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    match publisher.get_batch(start).await {
        Ok(Some(batch)) => Ok(Json(batch)),
        Ok(None) => Err((StatusCode::NOT_FOUND, format!("Batch {start} not found"))),
        Err(e) => {
            error!(start, error = %e, "Failed to get batch");
            Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
        }
    }
}

/// Get message by sequence number
async fn get_message_handler(
    State(publisher): State<Arc<LocalPublisher>>,
    Path(sequence): Path<u64>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    match publisher.get(sequence).await {
        Ok(Some(message)) => Ok(Json(message)),
        Ok(None) => Err((StatusCode::NOT_FOUND, format!("Message {sequence} not found"))),
        Err(e) => {
            error!(sequence, error = %e, "Failed to get message");
            Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
        }
    }
}

/// Get latest published sequence number
async fn get_latest_handler(
    State(publisher): State<Arc<LocalPublisher>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    match publisher.get_latest_sequence().await {
        Ok(sequence) => Ok(Json(LatestSequenceResponse { sequence })),
        Err(e) => {
            error!(error = %e, "Failed to get latest sequence");
            Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::keccak256;
    use synddb_shared::types::message::MessageType;

    const TEST_PRIVATE_KEY: &str =
        "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

    fn test_signer() -> Arc<MessageSigner> {
        Arc::new(MessageSigner::new(TEST_PRIVATE_KEY).unwrap())
    }

    /// Create a properly signed message for testing
    async fn create_signed_message(
        signer: &MessageSigner,
        sequence: u64,
        timestamp: u64,
    ) -> SignedMessage {
        let payload = b"test payload";
        let message_hash = keccak256(payload);
        let signing_payload =
            SignedMessage::compute_signing_payload(sequence, timestamp, message_hash);
        let signature = signer.sign(signing_payload).await.unwrap();

        SignedMessage {
            sequence,
            timestamp,
            message_type: MessageType::Changeset,
            payload: payload.to_vec(),
            message_hash: format!("0x{}", hex::encode(message_hash)),
            signature: signature.to_hex_prefixed(),
            signer: format!("{:?}", signer.address()),
        }
    }

    #[tokio::test]
    async fn test_local_publisher_roundtrip() {
        let signer = test_signer();
        let publisher = LocalPublisher::new(LocalConfig::in_memory(), Arc::clone(&signer)).unwrap();

        let message = create_signed_message(&signer, 1, 1700000000).await;

        // Publish
        let result = publisher.publish(&message).await;
        assert!(result.success, "Publish failed: {:?}", result.error);
        assert_eq!(result.reference, Some("local://batches/1_1".to_string()));

        // Retrieve message
        let retrieved = publisher.get(1).await.unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().sequence, 1);

        // Retrieve batch
        let batch = publisher.get_batch(1).await.unwrap();
        assert!(batch.is_some());
        let batch = batch.unwrap();
        assert_eq!(batch.start_sequence, 1);
        assert_eq!(batch.end_sequence, 1);
        assert_eq!(batch.messages.len(), 1);

        // Check latest sequence
        let latest = publisher.get_latest_sequence().await.unwrap();
        assert_eq!(latest, Some(1));
    }

    #[tokio::test]
    async fn test_local_publisher_batch() {
        let signer = test_signer();
        let publisher = LocalPublisher::new(LocalConfig::in_memory(), Arc::clone(&signer)).unwrap();

        // Create messages
        let msg1 = create_signed_message(&signer, 1, 1700000000).await;
        let msg2 = create_signed_message(&signer, 2, 1700000001).await;
        let messages = vec![msg1, msg2];

        // Create properly signed batch
        let messages_hash = SignedBatch::compute_messages_hash(&messages).unwrap();
        let batch_payload = SignedBatch::compute_signing_payload(1, 2, messages_hash);
        let batch_sig = signer.sign(batch_payload).await.unwrap();

        let batch = SignedBatch {
            start_sequence: 1,
            end_sequence: 2,
            messages,
            batch_signature: batch_sig.to_hex_prefixed(),
            signer: format!("{:?}", signer.address()),
            created_at: 1700000002,
        };

        // Publish batch
        let result = publisher.publish_batch(&batch).await;
        assert!(result.success, "Publish failed: {:?}", result.error);

        // Retrieve individual messages
        let msg1 = publisher.get(1).await.unwrap();
        assert!(msg1.is_some());
        assert_eq!(msg1.unwrap().sequence, 1);

        let msg2 = publisher.get(2).await.unwrap();
        assert!(msg2.is_some());
        assert_eq!(msg2.unwrap().sequence, 2);

        // Latest should be 2
        assert_eq!(publisher.get_latest_sequence().await.unwrap(), Some(2));
    }

    #[tokio::test]
    async fn test_local_publisher_not_found() {
        let signer = test_signer();
        let publisher = LocalPublisher::new(LocalConfig::in_memory(), Arc::clone(&signer)).unwrap();

        // No messages published yet
        let msg = publisher.get(999).await.unwrap();
        assert!(msg.is_none());

        let batch = publisher.get_batch(999).await.unwrap();
        assert!(batch.is_none());

        let latest = publisher.get_latest_sequence().await.unwrap();
        assert!(latest.is_none());
    }

    #[tokio::test]
    async fn test_local_publisher_state_persistence() {
        let signer = test_signer();
        let publisher = LocalPublisher::new(LocalConfig::in_memory(), Arc::clone(&signer)).unwrap();

        // Initially no state
        assert!(publisher.load_state().await.unwrap().is_none());

        // Publish a message
        let message = create_signed_message(&signer, 42, 1700000000).await;
        publisher.publish(&message).await;

        // State should now reflect the published message
        assert_eq!(publisher.load_state().await.unwrap(), Some(42));
    }

    #[tokio::test]
    async fn test_local_publisher_sequence_zero() {
        let signer = test_signer();
        let publisher =
            LocalPublisher::new(LocalConfig::in_memory(), Arc::clone(&signer)).unwrap();

        let message = create_signed_message(&signer, 0, 1700000000).await;

        // Publish
        let result = publisher.publish(&message).await;
        assert!(result.success, "Publish failed: {:?}", result.error);
        assert_eq!(result.reference, Some("local://batches/0_0".to_string()));

        // Retrieve message
        let retrieved = publisher.get(0).await.unwrap();
        assert!(
            retrieved.is_some(),
            "Could not retrieve message at sequence 0"
        );
        assert_eq!(retrieved.unwrap().sequence, 0);

        // Check latest sequence
        let latest = publisher.get_latest_sequence().await.unwrap();
        assert_eq!(latest, Some(0));
    }
}
