//! Transport layer abstraction for CBOR batch publication
//!
//! This module defines a trait-based abstraction for publishing `CborBatch` to various
//! storage backends. The transport layer is separate from the batch format:
//!
//! - **GCS:** Stores raw CBOR+zstd bytes directly
//! - **Local:** In-memory or `SQLite` storage for testing
//! - **Arweave (future):** Wraps CBOR in ANS-104 `DataItem` with discovery tags
//!
//! The `CborBatch` format is transport-agnostic. Content hashes enable cross-system
//! addressing regardless of transport layer.

use async_trait::async_trait;
use std::fmt::Debug;
use synddb_shared::types::cbor::{batch::CborBatch, error::CborError, message::CborSignedMessage};

/// Information about a published or available batch
#[derive(Debug, Clone)]
pub struct BatchInfo {
    /// First sequence number in batch (inclusive)
    pub start_sequence: u64,
    /// Last sequence number in batch (inclusive)
    pub end_sequence: u64,
    /// Transport-specific reference (GCS path, Arweave TX ID, etc.)
    pub reference: String,
    /// Content hash of the batch (SHA-256)
    pub content_hash: [u8; 32],
}

/// Metadata returned after successful batch publication
#[derive(Debug, Clone)]
pub struct PublishMetadata {
    /// Transport-specific reference (GCS path, Arweave TX ID, etc.)
    pub reference: String,
    /// Content hash of the published batch
    pub content_hash: [u8; 32],
    /// Compressed size in bytes
    pub compressed_bytes: usize,
    /// Uncompressed size in bytes
    pub uncompressed_bytes: usize,
}

/// Errors that can occur during transport operations
#[derive(Debug, thiserror::Error)]
pub enum TransportError {
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

    #[error("Signature verification failed: {0}")]
    SignatureVerification(String),
}

impl From<CborError> for TransportError {
    fn from(e: CborError) -> Self {
        Self::Serialization(e.to_string())
    }
}

/// Trait for transport layer implementations
///
/// Transport layers handle the storage and retrieval of `CborBatch` data.
/// The batch format is transport-agnostic - different transports may wrap
/// the CBOR data differently (e.g., ANS-104 for Arweave) but the core
/// `CborBatch` structure remains consistent.
#[async_trait]
pub trait TransportPublisher: Send + Sync + Debug {
    /// Transport name (e.g., "gcs", "arweave")
    fn name(&self) -> &str;

    /// Publish a CBOR batch to storage
    ///
    /// The batch is serialized to CBOR and compressed with zstd before upload.
    /// Returns metadata about the published batch including the storage reference.
    async fn publish(&self, batch: &CborBatch) -> Result<PublishMetadata, TransportError>;

    /// Fetch a batch by its start sequence number
    ///
    /// Returns `None` if no batch starting at this sequence exists.
    /// The returned batch has been decompressed and deserialized but
    /// signatures have NOT been verified - caller should verify if needed.
    async fn fetch(&self, start_sequence: u64) -> Result<Option<CborBatch>, TransportError>;

    /// List available batches in storage
    ///
    /// Returns batch metadata sorted by `start_sequence` ascending.
    async fn list_batches(&self) -> Result<Vec<BatchInfo>, TransportError>;

    /// Get the latest published sequence number
    ///
    /// Returns `None` if no batches have been published yet.
    async fn get_latest_sequence(&self) -> Result<Option<u64>, TransportError>;

    /// Get a single message by sequence number
    ///
    /// Searches batches to find the one containing this sequence, then
    /// extracts the specific message. Returns `None` if no batch contains
    /// this sequence.
    ///
    /// Note: This is O(n) in the number of batches for most implementations.
    /// For sequential replay, prefer iterating batches directly via `list_batches`
    /// and `fetch`.
    async fn get_message(&self, sequence: u64)
        -> Result<Option<CborSignedMessage>, TransportError>;
}
