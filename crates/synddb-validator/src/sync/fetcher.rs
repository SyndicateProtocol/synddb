//! Storage Fetcher trait for retrieving messages from storage layers

use anyhow::Result;
use async_trait::async_trait;
use std::fmt::Debug;
use synddb_shared::types::message::{SignedBatch, SignedMessage};

// Re-export BatchInfo from shared crate for backwards compatibility
pub use synddb_shared::types::batch::BatchInfo;

/// Trait for fetching messages from a storage layer
///
/// This is the read-only counterpart to `StoragePublisher` in the sequencer.
/// Implementations fetch `SignedMessage` payloads that were published by the sequencer.
///
/// # Batch Support
///
/// Fetchers can optionally support batch fetching for more efficient sequential sync.
/// When `supports_batches()` returns true, the validator can:
/// 1. Call `list_batches()` once at startup to build an in-memory index
/// 2. Iterate through batches sequentially using `get_batch_by_path()`
/// 3. Periodically refresh the index to discover new batches
///
/// This avoids O(n) list operations per message and enables efficient gap detection.
#[async_trait]
pub trait StorageFetcher: Send + Sync + Debug {
    /// Name of this fetcher (e.g., "gcs", "celestia", "http")
    fn name(&self) -> &str;

    /// Whether this fetcher supports batch operations.
    ///
    /// When true, the validator can use `list_batches()` and `get_batch_by_path()`
    /// for more efficient sequential sync. When false, it falls back to single-message
    /// fetching via `get()`.
    ///
    /// Default: false (backwards compatible)
    fn supports_batches(&self) -> bool {
        false
    }

    // =========================================================================
    // Single-message operations (always available)
    // =========================================================================

    /// Fetch a signed message by sequence number
    ///
    /// Returns `Ok(Some(message))` if found, `Ok(None)` if not found,
    /// or `Err` on network/parsing errors.
    ///
    /// For fetchers that support batches, this may internally find the batch
    /// containing the sequence and extract the message.
    async fn get(&self, sequence: u64) -> Result<Option<SignedMessage>>;

    /// Get the latest sequence number available from this storage layer
    ///
    /// Returns `Ok(Some(sequence))` if messages exist, `Ok(None)` if empty,
    /// or `Err` on network errors.
    async fn get_latest_sequence(&self) -> Result<Option<u64>>;

    // =========================================================================
    // Batch operations (optional, for efficient sequential sync)
    // =========================================================================

    /// List all available batches, sorted by start sequence.
    ///
    /// Returns a vector of `BatchInfo` containing sequence ranges and paths.
    /// The validator caches this list to avoid repeated list operations.
    ///
    /// Default: returns empty vec (no batch support)
    async fn list_batches(&self) -> Result<Vec<BatchInfo>> {
        Ok(vec![])
    }

    /// Fetch a batch by its start sequence number.
    ///
    /// Returns `Ok(Some(batch))` if found, `Ok(None)` if not found,
    /// or `Err` on network/parsing errors.
    ///
    /// Default: returns None (no batch support)
    async fn get_batch(&self, _start_sequence: u64) -> Result<Option<SignedBatch>> {
        Ok(None)
    }

    /// Fetch a batch by its path (from `BatchInfo.path`).
    ///
    /// This is more efficient than `get_batch()` when iterating through
    /// a cached batch index, as it avoids path construction.
    ///
    /// Default: returns None (no batch support)
    async fn get_batch_by_path(&self, _path: &str) -> Result<Option<SignedBatch>> {
        Ok(None)
    }
}

// Tests for BatchInfo are in synddb-shared::types::batch
