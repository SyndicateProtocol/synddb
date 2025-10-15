//! Local write framework for SyndDB
//!
//! This module provides the core framework for handling local write operations
//! in the sequencer, including validation, execution, and queuing for blockchain
//! submission.

use crate::database::SyndDatabase;
use crate::extensions::ExtensionRegistry;
use crate::types::*;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

// ============================================================================
// Local Write Processor
// ============================================================================

/// Processes local write operations using registered extensions
pub struct LocalWriteProcessor {
    /// Extension registry
    registry: Arc<ExtensionRegistry>,
    /// Database handle
    database: Arc<dyn SyndDatabase>,
    /// Queue for writes awaiting blockchain submission
    submit_queue: Arc<RwLock<ChainSubmitQueue>>,
    /// Monotonic nonce counter
    nonce_counter: Arc<RwLock<u64>>,
}

impl LocalWriteProcessor {
    /// Create a new local write processor
    pub fn new(registry: Arc<ExtensionRegistry>, database: Arc<dyn SyndDatabase>) -> Self {
        Self {
            registry,
            database,
            submit_queue: Arc::new(RwLock::new(ChainSubmitQueue::new())),
            nonce_counter: Arc::new(RwLock::new(0)),
        }
    }

    /// Execute a local write operation
    ///
    /// This provides ultra-low latency (<1ms) by executing immediately in the
    /// sequencer's local SQLite database without distributed consensus.
    pub async fn execute_local_write(&self, mut write: LocalWrite) -> Result<LocalWriteReceipt> {
        let start = std::time::Instant::now();

        // Assign nonce if not present
        if write.nonce == 0 {
            let mut nonce = self.nonce_counter.write().await;
            *nonce += 1;
            write.nonce = *nonce;
        }

        // Find the registered extension for this write type
        let extension = self
            .registry
            .get_write_extension(&write.write_type)
            .ok_or_else(|| {
                Error::InvalidOperation(format!("Unknown write type: {}", write.write_type))
            })?;

        debug!(
            "Processing local write: type={}, nonce={}",
            write.write_type, write.nonce
        );

        // Validate using extension
        extension.validate(&write.request)?;

        // Pre-execution hook
        extension.pre_execute(&write.request).await?;

        // Convert to SQL using extension
        let sql_operations = extension.to_sql(&write.request)?;

        // Execute in local SQLite
        let result = self.database.execute_batch(sql_operations).await?;

        // Post-execution hook
        extension.post_execute(&write.request, &result).await?;

        // Queue for blockchain submission
        self.submit_queue.write().await.enqueue(write.clone())?;

        let latency = start.elapsed();

        info!(
            "Local write executed: type={}, nonce={}, latency={:?}",
            write.write_type, write.nonce, latency
        );

        Ok(LocalWriteReceipt {
            write_id: generate_write_id(),
            status: LocalWriteStatus::CommittedLocally,
            latency,
            replication_eta: "~1s".to_string(),
        })
    }

    /// Get writes pending blockchain submission
    pub async fn get_pending_writes(&self) -> Vec<LocalWrite> {
        self.submit_queue.read().await.get_pending()
    }

    /// Mark writes as submitted to blockchain
    pub async fn mark_submitted(&self, write_ids: &[String]) -> Result<()> {
        self.submit_queue.write().await.mark_submitted(write_ids)
    }

    /// Get the current nonce
    pub async fn get_nonce(&self) -> u64 {
        *self.nonce_counter.read().await
    }
}

// ============================================================================
// Chain Submit Queue
// ============================================================================

/// Queue of local writes awaiting blockchain submission
pub struct ChainSubmitQueue {
    /// Pending writes
    pending: Vec<QueuedWrite>,
    /// Maximum queue size before backpressure
    max_size: usize,
}

impl ChainSubmitQueue {
    /// Create a new chain submit queue
    pub fn new() -> Self {
        Self {
            pending: Vec::new(),
            max_size: 10000,
        }
    }

    /// Create a queue with custom max size
    pub fn with_max_size(max_size: usize) -> Self {
        Self {
            pending: Vec::new(),
            max_size,
        }
    }

    /// Enqueue a write for blockchain submission
    pub fn enqueue(&mut self, write: LocalWrite) -> Result<()> {
        if self.pending.len() >= self.max_size {
            warn!(
                "Chain submit queue full ({}), applying backpressure",
                self.max_size
            );
            return Err(Error::InvalidOperation(
                "Chain submit queue full".to_string(),
            ));
        }

        self.pending.push(QueuedWrite {
            write,
            queued_at: crate::types::current_timestamp_ms(),
            submitted: false,
        });

        Ok(())
    }

    /// Get all pending writes
    pub fn get_pending(&self) -> Vec<LocalWrite> {
        self.pending
            .iter()
            .filter(|w| !w.submitted)
            .map(|w| w.write.clone())
            .collect()
    }

    /// Get a batch of pending writes
    pub fn get_batch(&self, size: usize) -> Vec<LocalWrite> {
        self.pending
            .iter()
            .filter(|w| !w.submitted)
            .take(size)
            .map(|w| w.write.clone())
            .collect()
    }

    /// Mark writes as submitted to blockchain
    ///
    /// Note: Currently marks all pending writes as submitted since LocalWrite
    /// doesn't store write_id. In production, LocalWrite should include a write_id
    /// field to enable selective marking.
    pub fn mark_submitted(&mut self, _write_ids: &[String]) -> Result<()> {
        // TODO: Add write_id field to LocalWrite to enable selective marking
        // For now, mark all pending writes as submitted
        for queued in &mut self.pending {
            queued.submitted = true;
        }
        Ok(())
    }

    /// Get queue size
    pub fn len(&self) -> usize {
        self.pending.len()
    }

    /// Check if queue is empty
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// Clear submitted writes from queue
    pub fn clear_submitted(&mut self) {
        self.pending.retain(|w| !w.submitted);
    }
}

impl Default for ChainSubmitQueue {
    fn default() -> Self {
        Self::new()
    }
}

/// A write in the submission queue
#[derive(Debug, Clone)]
struct QueuedWrite {
    /// The write itself
    write: LocalWrite,
    /// When it was queued (for future monitoring/analytics)
    #[allow(dead_code)]
    queued_at: u64,
    /// Whether it has been submitted to blockchain
    submitted: bool,
}

// ============================================================================
// Write Builder
// ============================================================================

/// Builder for constructing local writes
pub struct LocalWriteBuilder {
    write_type: String,
    request: serde_json::Value,
    timestamp: Option<u64>,
    nonce: Option<u64>,
}

impl LocalWriteBuilder {
    /// Create a new write builder
    pub fn new(write_type: impl Into<String>) -> Self {
        Self {
            write_type: write_type.into(),
            request: serde_json::Value::Null,
            timestamp: None,
            nonce: None,
        }
    }

    /// Set the request payload
    pub fn request(mut self, request: serde_json::Value) -> Self {
        self.request = request;
        self
    }

    /// Set the timestamp
    pub fn timestamp(mut self, timestamp: u64) -> Self {
        self.timestamp = Some(timestamp);
        self
    }

    /// Set the nonce
    pub fn nonce(mut self, nonce: u64) -> Self {
        self.nonce = Some(nonce);
        self
    }

    /// Build the local write
    pub fn build(self) -> LocalWrite {
        let timestamp = self
            .timestamp
            .unwrap_or_else(crate::types::current_timestamp_ms);

        LocalWrite {
            write_type: self.write_type,
            request: self.request,
            timestamp,
            nonce: self.nonce.unwrap_or(0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_write_builder() {
        let write = LocalWriteBuilder::new("test_write")
            .request(serde_json::json!({"key": "value"}))
            .nonce(42)
            .build();

        assert_eq!(write.write_type, "test_write");
        assert_eq!(write.nonce, 42);
    }

    #[test]
    fn test_chain_submit_queue() {
        let mut queue = ChainSubmitQueue::new();

        let write = LocalWriteBuilder::new("test")
            .request(serde_json::json!({}))
            .build();

        assert!(queue.enqueue(write).is_ok());
        assert_eq!(queue.len(), 1);

        let pending = queue.get_pending();
        assert_eq!(pending.len(), 1);
    }
}
