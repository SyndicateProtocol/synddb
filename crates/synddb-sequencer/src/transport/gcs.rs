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
use google_cloud_storage::client::Client;
use serde::{Deserialize, Serialize};
use synddb_shared::types::cbor::{batch::CborBatch, message::CborSignedMessage};
use tracing::{debug, info, warn};

/// Configuration for GCS transport
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GcsTransportConfig {
    /// GCS bucket name
    pub bucket: String,
    /// Path prefix within the bucket (default: "sequencer")
    pub prefix: String,
    /// GCS emulator host URL for local testing
    pub emulator_host: Option<String>,
}

impl GcsTransportConfig {
    pub fn new(bucket: impl Into<String>) -> Self {
        Self {
            bucket: bucket.into(),
            prefix: "sequencer".to_string(),
            emulator_host: None,
        }
    }

    pub fn with_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.prefix = prefix.into();
        self
    }

    pub fn with_emulator_host(mut self, host: impl Into<String>) -> Self {
        let host = host.into();
        self.emulator_host = if host.is_empty() { None } else { Some(host) };
        self
    }
}

/// Google Cloud Storage transport for CBOR batches
pub struct GcsTransport {
    client: Client,
    config: GcsTransportConfig,
}

impl std::fmt::Debug for GcsTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GcsTransport")
            .field("bucket", &self.config.bucket)
            .field("prefix", &self.config.prefix)
            .field("emulator", &self.config.emulator_host.is_some())
            .finish()
    }
}

impl GcsTransport {
    /// Create a new GCS transport
    ///
    /// Uses default credentials or emulator if configured.
    pub async fn new(config: GcsTransportConfig) -> Result<Self, TransportError> {
        use google_cloud_storage::client::ClientConfig;

        let client_config = if let Some(ref emulator_host) = config.emulator_host {
            info!(emulator_host = %emulator_host, "Using GCS emulator for transport");
            let mut cfg = ClientConfig::default().anonymous();
            cfg.storage_endpoint = emulator_host.clone();
            cfg
        } else {
            ClientConfig::default()
                .with_auth()
                .await
                .map_err(|e| TransportError::Config(format!("Failed to configure GCS auth: {e}")))?
        };

        let client = Client::new(client_config);

        info!(
            bucket = %config.bucket,
            prefix = %config.prefix,
            "GCS transport initialized"
        );

        Ok(Self { client, config })
    }

    /// Get the path for a batch file
    fn batch_path(&self, start_sequence: u64, end_sequence: u64) -> String {
        format!(
            "{}/batches/{:012}_{:012}.cbor.zst",
            self.config.prefix, start_sequence, end_sequence
        )
    }

    /// Parse a batch filename to extract start and end sequence numbers
    ///
    /// Expected format: `{start:012}_{end:012}.cbor.zst`
    pub(super) fn parse_batch_filename(filename: &str) -> Option<(u64, u64)> {
        let without_ext = filename.strip_suffix(".cbor.zst")?;
        let mut parts = without_ext.split('_');
        let start = parts.next()?.parse::<u64>().ok()?;
        let end = parts.next()?.parse::<u64>().ok()?;
        if parts.next().is_some() {
            return None;
        }
        Some((start, end))
    }

    /// Upload data to GCS
    async fn upload(
        &self,
        path: &str,
        data: Vec<u8>,
        content_type: &str,
    ) -> Result<(), TransportError> {
        use google_cloud_storage::http::objects::upload::{Media, UploadObjectRequest, UploadType};

        let mut media = Media::new(path.to_string());
        media.content_type = content_type.to_string().into();

        let upload_type = UploadType::Simple(media);
        let request = UploadObjectRequest {
            bucket: self.config.bucket.clone(),
            ..Default::default()
        };

        self.client
            .upload_object(&request, data, &upload_type)
            .await
            .map_err(|e| TransportError::Storage(format!("Failed to upload to GCS: {e}")))?;

        Ok(())
    }

    /// Download data from GCS
    async fn download(&self, path: &str) -> Result<Option<Vec<u8>>, TransportError> {
        use google_cloud_storage::http::objects::{download::Range, get::GetObjectRequest};

        let request = GetObjectRequest {
            bucket: self.config.bucket.clone(),
            object: path.to_string(),
            ..Default::default()
        };

        match self
            .client
            .download_object(&request, &Range::default())
            .await
        {
            Ok(data) => Ok(Some(data)),
            Err(e) => {
                let error_str = e.to_string();
                if error_str.contains("404") || error_str.contains("No such object") {
                    Ok(None)
                } else {
                    Err(TransportError::Storage(format!(
                        "Failed to download from GCS: {e}"
                    )))
                }
            }
        }
    }

    /// Find the batch containing a specific sequence number
    ///
    /// Returns the path of the batch file if found.
    ///
    /// TODO: Performance note
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
    async fn list_batch_files(&self) -> Result<Vec<(u64, u64, String)>, TransportError> {
        use google_cloud_storage::http::objects::list::ListObjectsRequest;

        let prefix = format!("{}/batches/", self.config.prefix);
        let request = ListObjectsRequest {
            bucket: self.config.bucket.clone(),
            prefix: Some(prefix),
            ..Default::default()
        };

        match self.client.list_objects(&request).await {
            Ok(response) => {
                let mut batches = Vec::new();
                for obj in response.items.unwrap_or_default() {
                    if let Some(filename) = obj.name.rsplit('/').next() {
                        if let Some((start, end)) = Self::parse_batch_filename(filename) {
                            batches.push((start, end, obj.name));
                        }
                    }
                }
                batches.sort_by_key(|(start, _, _)| *start);
                Ok(batches)
            }
            Err(e) => Err(TransportError::Storage(format!(
                "Failed to list batch objects: {e}"
            ))),
        }
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

        let mut infos = Vec::with_capacity(batch_files.len());
        for (start, end, path) in batch_files {
            // We need to fetch each batch to get the content hash
            // For efficiency, we could store metadata separately, but for now
            // we just return placeholder hashes and let callers fetch if needed
            infos.push(BatchInfo {
                start_sequence: start,
                end_sequence: end,
                reference: format!("gs://{}/{}", self.config.bucket, path),
                content_hash: [0u8; 32], // Placeholder - fetch batch to get real hash
            });
        }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_batch_filename_valid() {
        let result = GcsTransport::parse_batch_filename("000000000001_000000000050.cbor.zst");
        assert_eq!(result, Some((1, 50)));

        let result = GcsTransport::parse_batch_filename("000000001000_000000002000.cbor.zst");
        assert_eq!(result, Some((1000, 2000)));

        let result = GcsTransport::parse_batch_filename("000000000042_000000000042.cbor.zst");
        assert_eq!(result, Some((42, 42)));
    }

    #[test]
    fn test_parse_batch_filename_invalid() {
        // Wrong extension
        assert_eq!(
            GcsTransport::parse_batch_filename("000000000001_000000000050.json"),
            None
        );

        // Missing extension
        assert_eq!(
            GcsTransport::parse_batch_filename("000000000001_000000000050"),
            None
        );

        // Extra underscore
        assert_eq!(
            GcsTransport::parse_batch_filename("000000000001_000000000050_extra.cbor.zst"),
            None
        );

        // Non-numeric
        assert_eq!(
            GcsTransport::parse_batch_filename("abcdef_ghijkl.cbor.zst"),
            None
        );
    }

    #[test]
    fn test_batch_filename_sorting() {
        let mut filenames = vec![
            "000000000051_000000000100.cbor.zst",
            "000000000001_000000000050.cbor.zst",
            "000000000101_000000000150.cbor.zst",
        ];
        filenames.sort();

        assert_eq!(
            filenames,
            vec![
                "000000000001_000000000050.cbor.zst",
                "000000000051_000000000100.cbor.zst",
                "000000000101_000000000150.cbor.zst",
            ]
        );
    }
}
