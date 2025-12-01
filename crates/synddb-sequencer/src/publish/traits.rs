//! `DAPublisher` trait and related types

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::fmt::Debug;

use synddb_shared::types::SignedMessage;

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
#[async_trait]
pub trait DAPublisher: Send + Sync + Debug {
    /// Name of this publisher (e.g., "gcs", "celestia")
    fn name(&self) -> &str;

    /// Publish a signed message
    ///
    /// Returns a reference to the published data that can be used for retrieval.
    async fn publish(&self, message: &SignedMessage) -> PublishResult;

    /// Retrieve a signed message by sequence number
    ///
    /// Returns None if the message doesn't exist.
    async fn get(&self, sequence: u64) -> Result<Option<SignedMessage>, PublishError>;

    /// Get the latest sequence number that has been published
    ///
    /// Returns None if no messages have been published yet.
    async fn get_latest_sequence(&self) -> Result<Option<u64>, PublishError>;

    /// Save the current sequence state for recovery
    async fn save_state(&self, sequence: u64) -> Result<(), PublishError>;

    /// Load the sequence state (for recovery on startup)
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
