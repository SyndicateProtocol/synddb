//! Google Cloud Storage transport for CBOR batches
//!
//! Stores CBOR batches in GCS with `zstd` compression:
//!
//! ```text
//! gs://{bucket}/{prefix}/
//! └── batches/
//!     ├── 000000000001_000000000050.cbor.zst   # messages 1-50
//!     ├── 000000000051_000000000100.cbor.zst   # messages 51-100
//!     └── ...
//! ```
//!
//! Batch filenames follow the pattern `{start:012}_{end:012}.cbor.zst` where:
//! - `start` is the first sequence number in the batch (inclusive)
//! - `end` is the last sequence number in the batch (inclusive)
//! - Both are zero-padded to 12 digits (supports ~1 trillion sequences)

use crate::transport::traits::{BatchInfo, PublishMetadata, TransportError, TransportPublisher};
use async_trait::async_trait;
use synddb_shared::types::{
    batch::{format_batch_filename, parse_batch_filename},
    cbor::{batch::CborBatch, message::CborSignedMessage},
};
use synddb_storage::{StorageClient, StorageConfig};
use tracing::{debug, info, warn};

/// Configuration for GCS transport (re-exported from synddb-storage)
pub type GcsTransportConfig = synddb_storage::GcsConfig;

/// Google Cloud Storage transport for CBOR batches
pub struct GcsTransport {
    client: StorageClient,
}

impl std::fmt::Debug for GcsTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GcsTransport")
            .field("bucket", &self.client.bucket())
            .field("prefix", &self.client.prefix())
            .finish()
    }
}

impl GcsTransport {
    /// Create a new GCS transport
    ///
    /// Uses default credentials or emulator if configured.
    pub async fn new(config: GcsTransportConfig) -> Result<Self, TransportError> {
        let storage_config = StorageConfig::from(config);
        let client = StorageClient::new(storage_config)
            .await
            .map_err(|e| TransportError::Config(e.to_string()))?;

        info!(
            bucket = %client.bucket(),
            prefix = %client.prefix(),
            "GCS transport initialized"
        );

        Ok(Self { client })
    }

    /// Get the path for a batch file
    fn batch_path(&self, start_sequence: u64, end_sequence: u64) -> String {
        format!(
            "{}/batches/{}",
            self.client.prefix(),
            format_batch_filename(start_sequence, end_sequence)
        )
    }

    /// Upload data to GCS
    async fn upload(&self, path: &str, data: Vec<u8>) -> Result<(), TransportError> {
        self.client
            .write(path, data)
            .await
            .map_err(|e| TransportError::Storage(e.to_string()))?;
        Ok(())
    }

    /// Download data from GCS
    async fn download(&self, path: &str) -> Result<Option<Vec<u8>>, TransportError> {
        self.client
            .read(path)
            .await
            .map_err(|e| TransportError::Storage(e.to_string()))
    }

    /// Find the batch containing a specific sequence number
    ///
    /// Returns the path of the batch file if found.
    ///
    /// # Performance Note
    ///
    /// This is O(n) in the number of batches - it lists all batches and scans them.
    /// For large numbers of batches, consider:
    /// 1. Prefix narrowing: estimate batch start from sequence if batch sizes are consistent
    /// 2. In-memory caching: cache (start, end) -> filename after first list
    /// 3. For sequential replay, iterate batches in order by filename instead of using `get_message()`
    async fn find_batch_containing(&self, sequence: u64) -> Result<Option<String>, TransportError> {
        let batches = self.list_batch_files().await?;
        for (start, end, path) in batches {
            if sequence >= start && sequence <= end {
                return Ok(Some(path));
            }
        }
        Ok(None)
    }

    /// List all batch files and parse their metadata
    ///
    /// Handles GCS pagination to retrieve all batches.
    async fn list_batch_files(&self) -> Result<Vec<(u64, u64, String)>, TransportError> {
        let prefix = format!("{}/batches/", self.client.prefix());
        let objects = self
            .client
            .list(&prefix)
            .await
            .map_err(|e| TransportError::Storage(e.to_string()))?;

        let mut batches = Vec::new();
        for obj in objects {
            if let Some(filename) = obj.name.rsplit('/').next() {
                if let Some((start, end)) = parse_batch_filename(filename) {
                    batches.push((start, end, obj.name));
                }
            }
        }

        batches.sort_by_key(|(start, _, _)| *start);
        Ok(batches)
    }
}

#[async_trait]
impl TransportPublisher for GcsTransport {
    fn name(&self) -> &str {
        "gcs"
    }

    async fn publish(&self, batch: &CborBatch) -> Result<PublishMetadata, TransportError> {
        let path = self.batch_path(batch.start_sequence, batch.end_sequence);

        debug!(
            start_sequence = batch.start_sequence,
            end_sequence = batch.end_sequence,
            message_count = batch.messages.len(),
            "Serializing batch for GCS upload"
        );

        // Serialize to CBOR with zstd compression
        let uncompressed = batch.to_cbor()?;
        let compressed = batch.to_cbor_zstd()?;

        let uncompressed_bytes = uncompressed.len();
        let compressed_bytes = compressed.len();
        let compression_ratio = uncompressed_bytes as f64 / compressed_bytes as f64;

        debug!(
            uncompressed_bytes = uncompressed_bytes,
            compressed_bytes = compressed_bytes,
            compression_ratio = format!("{:.2}x", compression_ratio),
            "Batch serialized"
        );

        // Upload to GCS
        self.upload(&path, compressed).await?;

        let reference = format!("gs://{}/{}", self.client.bucket(), path);

        info!(
            start_sequence = batch.start_sequence,
            end_sequence = batch.end_sequence,
            message_count = batch.messages.len(),
            compressed_bytes = compressed_bytes,
            compression_ratio = format!("{:.2}x", compression_ratio),
            reference = %reference,
            "Batch published to GCS"
        );

        Ok(PublishMetadata {
            reference,
            content_hash: batch.content_hash,
            compressed_bytes,
            uncompressed_bytes,
        })
    }

    async fn fetch(&self, start_sequence: u64) -> Result<Option<CborBatch>, TransportError> {
        // Find the batch file that starts with this sequence
        let batches = self.list_batch_files().await?;

        let batch_path = batches
            .iter()
            .find(|(start, _, _)| *start == start_sequence)
            .map(|(_, _, path)| path.clone());

        let Some(path) = batch_path else {
            debug!(start_sequence = start_sequence, "Batch not found");
            return Ok(None);
        };

        debug!(start_sequence = start_sequence, path = %path, "Fetching batch from GCS");

        let Some(data) = self.download(&path).await? else {
            warn!(start_sequence = start_sequence, path = %path, "Batch file missing");
            return Ok(None);
        };

        let compressed_bytes = data.len();

        // Decompress and parse
        let batch = CborBatch::from_cbor_zstd(&data)?;

        info!(
            start_sequence = batch.start_sequence,
            end_sequence = batch.end_sequence,
            message_count = batch.messages.len(),
            compressed_bytes = compressed_bytes,
            content_hash = %batch.content_hash_hex(),
            "Batch fetched from GCS"
        );

        Ok(Some(batch))
    }

    async fn list_batches(&self) -> Result<Vec<BatchInfo>, TransportError> {
        let batch_files = self.list_batch_files().await?;

        let infos = batch_files
            .into_iter()
            .map(|(start, end, path)| {
                BatchInfo::new(
                    start,
                    end,
                    format!("gs://{}/{}", self.client.bucket(), path),
                )
            })
            .collect();

        Ok(infos)
    }

    async fn get_latest_sequence(&self) -> Result<Option<u64>, TransportError> {
        let batches = self.list_batch_files().await?;
        Ok(batches.iter().map(|(_, end, _)| *end).max())
    }

    async fn get_message(
        &self,
        sequence: u64,
    ) -> Result<Option<CborSignedMessage>, TransportError> {
        let Some(path) = self.find_batch_containing(sequence).await? else {
            return Ok(None);
        };

        let Some(data) = self.download(&path).await? else {
            return Ok(None);
        };

        let batch = CborBatch::from_cbor_zstd(&data)?;

        // Find the message in this batch
        for msg in batch.messages {
            if msg.sequence().ok() == Some(sequence) {
                return Ok(Some(msg));
            }
        }

        Ok(None)
    }
}

// ============================================================================
// Tests
// ============================================================================

// Tests for parse_batch_filename and format_batch_filename are in synddb-shared
