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
use google_cloud_storage::client::{Storage, StorageControl};
use synddb_shared::{
    gcs::GcsConfig,
    types::{
        batch::{format_batch_filename, parse_batch_filename},
        cbor::{batch::CborBatch, message::CborSignedMessage},
    },
};
use tracing::{debug, info, warn};

/// Configuration for GCS transport (re-exported from synddb-shared)
pub type GcsTransportConfig = GcsConfig;

/// Google Cloud Storage transport for CBOR batches
pub struct GcsTransport {
    storage: Storage,
    control: StorageControl,
    config: GcsConfig,
}

impl std::fmt::Debug for GcsTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GcsTransport")
            .field("bucket", &self.config.bucket)
            .field("prefix", &self.config.prefix)
            .field("emulator", &self.config.is_emulator())
            .finish()
    }
}

impl GcsTransport {
    /// Create a new GCS transport
    ///
    /// Uses default credentials or emulator if configured.
    pub async fn new(config: GcsTransportConfig) -> Result<Self, TransportError> {
        let (storage, control) = if let Some(ref emulator_host) = config.emulator_host {
            info!(emulator_host = %emulator_host, "Using GCS emulator for transport");
            // For emulator, set the STORAGE_EMULATOR_HOST environment variable
            // The official SDK reads this automatically
            std::env::set_var("STORAGE_EMULATOR_HOST", emulator_host);

            let storage = Storage::builder().build().await.map_err(|e| {
                TransportError::Config(format!("Failed to create Storage client: {e}"))
            })?;

            let control = StorageControl::builder().build().await.map_err(|e| {
                TransportError::Config(format!("Failed to create StorageControl client: {e}"))
            })?;

            (storage, control)
        } else {
            let storage = Storage::builder().build().await.map_err(|e| {
                TransportError::Config(format!("Failed to create Storage client: {e}"))
            })?;

            let control = StorageControl::builder().build().await.map_err(|e| {
                TransportError::Config(format!("Failed to create StorageControl client: {e}"))
            })?;

            (storage, control)
        };

        info!(
            bucket = %config.bucket,
            prefix = %config.prefix,
            "GCS transport initialized"
        );

        Ok(Self {
            storage,
            control,
            config,
        })
    }

    /// Get the path for a batch file
    fn batch_path(&self, start_sequence: u64, end_sequence: u64) -> String {
        format!(
            "{}/batches/{}",
            self.config.prefix,
            format_batch_filename(start_sequence, end_sequence)
        )
    }

    /// Get the bucket name in the format expected by the SDK
    fn bucket_name(&self) -> &str {
        &self.config.bucket
    }

    /// Upload data to GCS
    async fn upload(
        &self,
        path: &str,
        data: Vec<u8>,
        _content_type: &str,
    ) -> Result<(), TransportError> {
        use bytes::Bytes;
        self.storage
            .write_object(self.bucket_name(), path, Bytes::from(data))
            .send_buffered()
            .await
            .map_err(|e| TransportError::Storage(format!("Failed to upload to GCS: {e}")))?;

        Ok(())
    }

    /// Download data from GCS
    async fn download(&self, path: &str) -> Result<Option<Vec<u8>>, TransportError> {
        let mut reader = match self
            .storage
            .read_object(self.bucket_name(), path)
            .send()
            .await
        {
            Ok(reader) => reader,
            Err(e) => {
                let error_str = e.to_string();
                if error_str.contains("404")
                    || error_str.contains("No such object")
                    || error_str.contains("not found")
                {
                    return Ok(None);
                } else {
                    return Err(TransportError::Storage(format!(
                        "Failed to download from GCS: {e}"
                    )));
                }
            }
        };

        // Read all chunks into a buffer
        let mut data = Vec::new();
        while let Some(chunk) = reader.next().await {
            let chunk = chunk.map_err(|e| {
                TransportError::Storage(format!("Failed to read chunk from GCS: {e}"))
            })?;
            data.extend_from_slice(&chunk);
        }

        Ok(Some(data))
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
        let prefix = format!("{}/batches/", self.config.prefix);
        let mut batches = Vec::new();
        let mut page_token: Option<String> = None;

        loop {
            let mut request = self
                .control
                .list_objects()
                .set_parent(self.bucket_name())
                .set_prefix(&prefix);

            if let Some(ref token) = page_token {
                request = request.set_page_token(token);
            }

            let response = request.send().await.map_err(|e| {
                TransportError::Storage(format!("Failed to list batch objects: {e}"))
            })?;

            // Parse batch info from object names
            for obj in response.objects {
                if let Some(filename) = obj.name.rsplit('/').next() {
                    if let Some((start, end)) = parse_batch_filename(filename) {
                        batches.push((start, end, obj.name));
                    }
                }
            }

            // Check for more pages
            if !response.next_page_token.is_empty() {
                debug!(token = %response.next_page_token, "Fetching next page of batch files");
                page_token = Some(response.next_page_token);
            } else {
                break;
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
        self.upload(&path, compressed, "application/cbor+zstd")
            .await?;

        let reference = format!("gs://{}/{}", self.config.bucket, path);

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
                BatchInfo::new(start, end, format!("gs://{}/{}", self.config.bucket, path))
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
