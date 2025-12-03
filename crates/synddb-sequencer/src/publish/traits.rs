//! `DAPublisher` trait and related types

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::fmt::Debug;

use synddb_shared::types::message::{SignedBatch, SignedMessage};

// TODO: Implement a batching layer that accumulates messages and flushes based on
// whichever threshold is hit first. This would sit between the HTTP handlers and
// the DAPublisher, providing efficient batching without changing the publisher interface.
//
// Two possible modes:
// - Fire-and-forget: HTTP handler returns immediately after sequencing, batch publishes async
// - Wait-for-batch: HTTP handler waits until its message's batch is published (higher latency)
//
// With larger batch sizes (e.g., 50+), it makes sense to add zstd compression to the
// batch before upload. Individual message payloads are already compressed, but the batch
// envelope and repeated JSON structure compress well at scale. Not worth it for batch_size=1.
//
// ```
// pub struct BatchConfig {
//     /// Maximum messages per batch before flushing (default: 50)
//     pub batch_size: usize,
//     /// Maximum time to wait before flushing a partial batch (default: 5s)
//     pub batch_interval: Duration,
//     /// Enable zstd compression for batches (recommended for batch_size > 1)
//     pub compress: bool,
// }
// ```

/// Result of publishing to a DA layer
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PublishResult {
    /// Name of the DA layer (e.g., "gcs", "celestia", "eigenda")
    pub layer: String,
    /// Whether the publish succeeded
    pub success: bool,
    /// Reference to the published data (e.g., GCS path, CID, blob ID)
    pub reference: Option<String>,
    /// Error message if publish failed
    pub error: Option<String>,
}

impl PublishResult {
    /// Create a successful publish result
    pub fn success(layer: impl Into<String>, reference: impl Into<String>) -> Self {
        Self {
            layer: layer.into(),
            success: true,
            reference: Some(reference.into()),
            error: None,
        }
    }

    /// Create a failed publish result
    pub fn failure(layer: impl Into<String>, error: impl Into<String>) -> Self {
        Self {
            layer: layer.into(),
            success: false,
            reference: None,
            error: Some(error.into()),
        }
    }
}

/// Trait for Data Availability publishers
///
/// Implement this trait to add support for new storage backends.
///
/// # Storage Layout
///
/// Messages are stored as atomic batches:
/// ```text
/// {prefix}/batches/{start:012}_{end:012}.json
/// ```
///
/// State is implicit in the batch filenames - the highest `end` sequence
/// across all batches represents the latest published state.
#[async_trait]
pub trait DAPublisher: Send + Sync + Debug {
    /// Name of this publisher (e.g., "gcs", "celestia")
    fn name(&self) -> &str;

    /// Publish a signed message
    ///
    /// This wraps the message in a single-message batch for atomic publication.
    async fn publish(&self, message: &SignedMessage) -> PublishResult;

    /// Publish a batch of messages atomically
    ///
    /// This is the primary publication method. Messages and state are published
    /// together in a single atomic operation.
    ///
    /// The batch is stored at: `{prefix}/batches/{start:012}_{end:012}.json`
    async fn publish_batch(&self, batch: &SignedBatch) -> PublishResult;

    /// Retrieve a signed message by sequence number
    ///
    /// Finds the batch containing this sequence and extracts the message.
    /// Returns None if the message doesn't exist.
    async fn get(&self, sequence: u64) -> Result<Option<SignedMessage>, PublishError>;

    /// Retrieve a batch by its start sequence number
    ///
    /// Returns None if the batch doesn't exist.
    async fn get_batch(&self, start_sequence: u64) -> Result<Option<SignedBatch>, PublishError>;

    /// Get the latest sequence number that has been published
    ///
    /// Scans batch filenames to find the highest `end` sequence.
    /// Returns None if no messages have been published yet.
    async fn get_latest_sequence(&self) -> Result<Option<u64>, PublishError>;

    /// Save the current sequence state (no-op for batch-based storage)
    ///
    /// State is implicit in batch filenames, so this is a no-op.
    /// Kept for API compatibility.
    async fn save_state(&self, sequence: u64) -> Result<(), PublishError>;

    /// Load the sequence state (for recovery on startup)
    ///
    /// Scans batch filenames to find the highest `end` sequence.
    async fn load_state(&self) -> Result<Option<u64>, PublishError>;
}

/// Errors that can occur during publishing
#[derive(Debug, thiserror::Error)]
pub enum PublishError {
    #[error("Storage error: {0}")]
    Storage(String),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Network error: {0}")]
    Network(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Configuration error: {0}")]
    Config(String),
}
