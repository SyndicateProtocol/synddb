//! Message batching layer for efficient storage publication
//!
//! The `Batcher` accumulates signed messages and publishes them as CBOR batches
//! to the configured transport layer. Batches are flushed to the storage layer when any threshold
//! is reached:
//! - Maximum message count
//! - Maximum batch size (bytes)
//! - Flush interval timeout
//! - Graceful shutdown trigger
//!
//! This provides efficient batching without blocking HTTP handlers.

use crate::{
    config::BatchConfig,
    transport::traits::{PublishMetadata, TransportError, TransportPublisher},
};
use k256::ecdsa::Signature;
use std::{sync::Arc, time::Instant};
use synddb_shared::{
    keys::EvmKeyManager,
    types::cbor::{
        batch::CborBatch, error::CborError, message::CborSignedMessage,
        verify::signature_from_bytes,
    },
};
use tokio::{
    sync::{mpsc, oneshot},
    time::{interval, Duration},
};
use tracing::{debug, error, info};

/// Statistics about batch operations
#[derive(Debug, Clone, Default)]
pub struct BatchStats {
    /// Total batches published
    pub batches_published: u64,
    /// Total messages published
    pub messages_published: u64,
    /// Total compressed bytes published
    pub bytes_published: u64,
    /// Total uncompressed bytes (for compression ratio calculation)
    pub bytes_uncompressed: u64,
    /// Current pending message count
    pub pending_messages: usize,
    /// Current pending byte count
    pub pending_bytes: usize,
    /// Last flush timestamp (epoch seconds)
    pub last_flush_timestamp: u64,
}

impl BatchStats {
    /// Calculate average compression ratio (uncompressed / compressed)
    pub fn compression_ratio(&self) -> f64 {
        if self.bytes_published == 0 {
            1.0
        } else {
            self.bytes_uncompressed as f64 / self.bytes_published as f64
        }
    }
}

/// Message to be batched
#[derive(Debug, Clone)]
struct PendingMessage {
    message: CborSignedMessage,
    size: usize,
}

/// Command sent to the batcher task
enum BatcherCommand {
    /// Add an already-signed CBOR message to the pending batch
    AddMessage {
        message: CborSignedMessage,
        response: oneshot::Sender<Result<(), BatcherError>>,
    },
    /// Force flush the current batch
    Flush {
        response: oneshot::Sender<Result<Option<PublishMetadata>, BatcherError>>,
    },
    /// Get current statistics
    GetStats {
        response: oneshot::Sender<BatchStats>,
    },
    /// Shutdown the batcher
    Shutdown,
}

/// Errors from the batcher
#[derive(Debug, thiserror::Error)]
pub enum BatcherError {
    #[error("Transport error: {0}")]
    Transport(#[from] TransportError),

    #[error("CBOR encoding error: {0}")]
    Cbor(#[from] CborError),

    #[error("Batcher channel closed")]
    ChannelClosed,

    #[error("Signing error: {0}")]
    Signing(String),
}

/// Handle for interacting with the batcher
#[derive(Clone, Debug)]
pub struct BatcherHandle {
    sender: mpsc::Sender<BatcherCommand>,
}

impl BatcherHandle {
    /// Add an already-signed CBOR message to the pending batch
    ///
    /// This may trigger an immediate flush if thresholds are exceeded.
    pub async fn add_message(&self, message: CborSignedMessage) -> Result<(), BatcherError> {
        let (tx, rx) = oneshot::channel();
        self.sender
            .send(BatcherCommand::AddMessage {
                message,
                response: tx,
            })
            .await
            .map_err(|_| BatcherError::ChannelClosed)?;
        rx.await.map_err(|_| BatcherError::ChannelClosed)?
    }

    /// Force flush the current batch immediately
    ///
    /// Returns metadata if a batch was published, None if no pending messages.
    pub async fn flush(&self) -> Result<Option<PublishMetadata>, BatcherError> {
        let (tx, rx) = oneshot::channel();
        self.sender
            .send(BatcherCommand::Flush { response: tx })
            .await
            .map_err(|_| BatcherError::ChannelClosed)?;
        rx.await.map_err(|_| BatcherError::ChannelClosed)?
    }

    /// Get current batch statistics
    pub async fn stats(&self) -> Result<BatchStats, BatcherError> {
        let (tx, rx) = oneshot::channel();
        self.sender
            .send(BatcherCommand::GetStats { response: tx })
            .await
            .map_err(|_| BatcherError::ChannelClosed)?;
        rx.await.map_err(|_| BatcherError::ChannelClosed)
    }

    /// Trigger graceful shutdown
    pub async fn shutdown(&self) -> Result<(), BatcherError> {
        self.sender
            .send(BatcherCommand::Shutdown)
            .await
            .map_err(|_| BatcherError::ChannelClosed)
    }
}

/// Batcher task that accumulates messages and publishes batches.
/// - Takes a `transport` trait object whose implementation can vary.
/// - On graceful shutdown, attempts to flush all pending messages to storage
#[derive(Clone, Debug)]
pub struct Batcher {
    config: BatchConfig,
    transport: Arc<dyn TransportPublisher>,
    key_manager: Arc<EvmKeyManager>,
    pending: Vec<PendingMessage>,
    pending_bytes: usize,
    stats: BatchStats,
    last_flush: Instant,
}

impl Batcher {
    /// Create a new batcher and spawn its background task.
    ///
    /// Returns a handle for sending messages to the batcher.
    ///
    /// ## Ordering Guarantee
    ///
    /// All commands are processed in FIFO order. The batcher uses an `mpsc` channel
    /// and a single-threaded event loop (`run`), ensuring that
    /// concurrent submissions are serialized. Messages are signed and batched in
    /// the order received, and batches are signed and published sequentially.
    /// This prevents race conditions even when multiple HTTP handlers submit
    /// messages simultaneously.
    pub fn spawn(
        config: BatchConfig,
        transport: Arc<dyn TransportPublisher>,
        key_manager: Arc<EvmKeyManager>,
    ) -> BatcherHandle {
        let (tx, rx) = mpsc::channel(1024);

        let batcher = Self {
            config: config.clone(),
            transport,
            key_manager,
            pending: Vec::new(),
            pending_bytes: 0,
            stats: BatchStats::default(),
            last_flush: Instant::now(),
        };

        tokio::spawn(batcher.run(rx, config.flush_interval));

        info!(
            max_messages = config.max_messages,
            max_bytes = config.max_batch_bytes,
            flush_interval_ms = config.flush_interval.as_millis(),
            "Batcher started"
        );

        BatcherHandle { sender: tx }
    }

    /// Run the batcher event loop
    async fn run(mut self, mut rx: mpsc::Receiver<BatcherCommand>, flush_interval: Duration) {
        let mut ticker = interval(flush_interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            tokio::select! {
                Some(cmd) = rx.recv() => {
                    match cmd {
                        BatcherCommand::AddMessage { message, response } => {
                            let result = self.handle_add_message(message).await;
                            let _ = response.send(result);
                        }
                        BatcherCommand::Flush { response } => {
                            let result = self.flush_batch().await;
                            let _ = response.send(result);
                        }
                        BatcherCommand::GetStats { response } => {
                            self.stats.pending_messages = self.pending.len();
                            self.stats.pending_bytes = self.pending_bytes;
                            let _ = response.send(self.stats.clone());
                        }
                        BatcherCommand::Shutdown => {
                            info!("Batcher shutting down, flushing pending messages");
                            if !self.pending.is_empty() {
                                if let Err(e) = self.flush_batch().await {
                                    error!(error = %e, "Failed to flush on shutdown");
                                }
                            }
                            break;
                        }
                    }
                }
                _ = ticker.tick() => {
                    if !self.pending.is_empty() && self.last_flush.elapsed() >= flush_interval {
                        debug!(
                            pending_messages = self.pending.len(),
                            pending_bytes = self.pending_bytes,
                            "Flush interval reached"
                        );
                        if let Err(e) = self.flush_batch().await {
                            error!(error = %e, "Interval flush failed");
                        }
                    }
                }
            }
        }

        info!(
            batches_published = self.stats.batches_published,
            messages_published = self.stats.messages_published,
            bytes_published = self.stats.bytes_published,
            "Batcher shutdown complete"
        );
    }

    /// Handle adding a message to the pending batch
    async fn handle_add_message(&mut self, message: CborSignedMessage) -> Result<(), BatcherError> {
        let size = message.size();

        self.pending.push(PendingMessage { message, size });
        self.pending_bytes += size;

        debug!(
            pending_messages = self.pending.len(),
            pending_bytes = self.pending_bytes,
            message_size = size,
            "Message added to batch"
        );

        // Check if we need to flush
        let should_flush = self.pending.len() >= self.config.max_messages
            || self.pending_bytes >= self.config.max_batch_bytes;

        if should_flush {
            let reason = if self.pending.len() >= self.config.max_messages {
                "max_messages"
            } else {
                "max_bytes"
            };
            debug!(
                reason = reason,
                pending_messages = self.pending.len(),
                pending_bytes = self.pending_bytes,
                "Threshold reached, flushing batch"
            );
            self.flush_batch().await?;
        }

        Ok(())
    }

    /// Flush the current batch to storage
    async fn flush_batch(&mut self) -> Result<Option<PublishMetadata>, BatcherError> {
        if self.pending.is_empty() {
            return Ok(None);
        }

        let messages: Vec<CborSignedMessage> =
            self.pending.drain(..).map(|pm| pm.message).collect();
        let message_count = messages.len();
        let pre_flush_bytes = self.pending_bytes;
        self.pending_bytes = 0;

        let created_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let signer_pubkey = self.key_manager.public_key();

        // Create and sign the batch
        let batch = CborBatch::new(messages, created_at, signer_pubkey, |data| {
            self.sign_batch_data(data)
        })?;

        let start_seq = batch.start_sequence;
        let end_seq = batch.end_sequence;

        info!(
            start_sequence = start_seq,
            end_sequence = end_seq,
            message_count = message_count,
            uncompressed_bytes = pre_flush_bytes,
            content_hash = %batch.content_hash_hex(),
            "Publishing batch"
        );

        // Publish to transport layer
        let metadata = self.transport.publish(&batch).await?;

        let compression_ratio =
            metadata.uncompressed_bytes as f64 / metadata.compressed_bytes as f64;

        info!(
            start_sequence = start_seq,
            end_sequence = end_seq,
            message_count = message_count,
            compressed_bytes = metadata.compressed_bytes,
            uncompressed_bytes = metadata.uncompressed_bytes,
            compression_ratio = format!("{:.2}x", compression_ratio),
            reference = %metadata.reference,
            "Batch published"
        );

        // Update stats
        self.stats.batches_published += 1;
        self.stats.messages_published += message_count as u64;
        self.stats.bytes_published += metadata.compressed_bytes as u64;
        self.stats.bytes_uncompressed += metadata.uncompressed_bytes as u64;
        self.stats.last_flush_timestamp = created_at;
        self.last_flush = Instant::now();

        Ok(Some(metadata))
    }

    /// Sign batch data synchronously.
    ///
    /// See [`spawn`](Self::spawn) for ordering guarantees.
    fn sign_batch_data(&self, data: &[u8]) -> Result<Signature, CborError> {
        use alloy::primitives::keccak256;

        let hash = keccak256(data);
        let sig = self
            .key_manager
            .sign_raw_sync(&hash.0)
            .map_err(|e| CborError::Signing(e.to_string()))?;

        let mut bytes = [0u8; 64];
        bytes[..32].copy_from_slice(&sig.r().to_be_bytes::<32>());
        bytes[32..].copy_from_slice(&sig.s().to_be_bytes::<32>());
        signature_from_bytes(&bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_batch_stats_compression_ratio() {
        let mut stats = BatchStats::default();
        assert_eq!(stats.compression_ratio(), 1.0);

        stats.bytes_published = 100;
        stats.bytes_uncompressed = 500;
        assert!((stats.compression_ratio() - 5.0).abs() < 0.001);
    }

    #[test]
    fn test_batch_config_default() {
        let config = BatchConfig::default();
        assert_eq!(config.max_messages, 50);
        assert_eq!(config.max_batch_bytes, 1_048_576);
        assert_eq!(config.flush_interval, Duration::from_secs(5));
    }
}
