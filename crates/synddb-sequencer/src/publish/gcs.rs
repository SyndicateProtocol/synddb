//! Google Cloud Storage publisher implementation
//!
//! Stores signed messages in GCS as atomic batches:
//!
//! ```text
//! gs://{bucket}/{prefix}/
//! └── batches/
//!     ├── 000000000001_000000000050.json   # messages 1-50
//!     ├── 000000000051_000000000100.json   # messages 51-100
//!     └── ...
//! ```
//!
//! Batch filenames follow the pattern `{start:012}_{end:012}.json` where:
//! - `start` is the first sequence number in the batch (inclusive)
//! - `end` is the last sequence number in the batch (inclusive)
//! - Both are zero-padded to 12 digits (supports ~1 trillion sequences)
//!
//! The latest published state is implicit: the highest `end` sequence
//! across all batch files. This ensures atomic publication of messages
//! with state, preventing partial publication failures.

use crate::publish::traits::{DAPublisher, PublishError, PublishResult};
use serde::{Deserialize, Serialize};

use async_trait::async_trait;
use synddb_shared::types::message::{SignedBatch, SignedMessage};
use tracing::{error, info, warn};

/// Configuration for GCS publisher
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GcsConfig {
    /// GCS bucket name
    pub bucket: String,
    /// Path prefix within the bucket (default: "sequencer")
    #[serde(default = "default_prefix")]
    pub prefix: String,
}

fn default_prefix() -> String {
    "sequencer".to_string()
}

impl GcsConfig {
    pub fn new(bucket: impl Into<String>) -> Self {
        Self {
            bucket: bucket.into(),
            prefix: default_prefix(),
        }
    }

    pub fn with_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.prefix = prefix.into();
        self
    }
}

/// Google Cloud Storage publisher
pub struct GcsPublisher {
    client: google_cloud_storage::client::Client,
    config: GcsConfig,
}

impl std::fmt::Debug for GcsPublisher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GcsPublisher")
            .field("config", &self.config)
            .field("client", &"<GCS Client>")
            .finish()
    }
}

impl GcsPublisher {
    /// Create a new GCS publisher
    ///
    /// Uses default credentials (`GOOGLE_APPLICATION_CREDENTIALS` env var,
    /// workload identity, or metadata server).
    pub async fn new(config: GcsConfig) -> Result<Self, PublishError> {
        use google_cloud_storage::client::{Client, ClientConfig};

        let client_config = ClientConfig::default()
            .with_auth()
            .await
            .map_err(|e| PublishError::Config(format!("Failed to configure GCS auth: {e}")))?;

        let client = Client::new(client_config);

        info!(bucket = %config.bucket, prefix = %config.prefix, "GCS publisher initialized");

        Ok(Self { client, config })
    }

    /// Get the path for a batch file
    ///
    /// Format: `{prefix}/batches/{start:012}_{end:012}.json`
    fn batch_path(&self, start_sequence: u64, end_sequence: u64) -> String {
        format!(
            "{}/batches/{:012}_{:012}.json",
            self.config.prefix, start_sequence, end_sequence
        )
    }

    /// Parse a batch filename to extract start and end sequence numbers
    ///
    /// Expected format: `{start:012}_{end:012}.json`
    /// Returns `Some((start, end))` if valid, `None` otherwise
    fn parse_batch_filename(filename: &str) -> Option<(u64, u64)> {
        let without_ext = filename.strip_suffix(".json")?;
        let mut parts = without_ext.split('_');
        let start = parts.next()?.parse::<u64>().ok()?;
        let end = parts.next()?.parse::<u64>().ok()?;
        // Ensure no extra parts
        if parts.next().is_some() {
            return None;
        }
        Some((start, end))
    }

    /// Helper to upload data to GCS
    async fn upload(&self, path: &str, data: Vec<u8>) -> Result<(), PublishError> {
        use google_cloud_storage::http::objects::upload::{Media, UploadObjectRequest, UploadType};

        let upload_type = UploadType::Simple(Media::new(path.to_string()));
        let request = UploadObjectRequest {
            bucket: self.config.bucket.clone(),
            ..Default::default()
        };

        self.client
            .upload_object(&request, data, &upload_type)
            .await
            .map_err(|e| PublishError::Storage(format!("Failed to upload to GCS: {e}")))?;

        Ok(())
    }

    /// Helper to download data from GCS
    async fn download(&self, path: &str) -> Result<Option<Vec<u8>>, PublishError> {
        use google_cloud_storage::http::objects::download::Range;
        use google_cloud_storage::http::objects::get::GetObjectRequest;

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
                    Err(PublishError::Storage(format!(
                        "Failed to download from GCS: {e}"
                    )))
                }
            }
        }
    }

    /// Get the latest sequence from batches directory
    async fn get_latest_batch_sequence(&self) -> Result<Option<u64>, PublishError> {
        use google_cloud_storage::http::objects::list::ListObjectsRequest;

        let prefix = format!("{}/batches/", self.config.prefix);
        let request = ListObjectsRequest {
            bucket: self.config.bucket.clone(),
            prefix: Some(prefix),
            ..Default::default()
        };

        match self.client.list_objects(&request).await {
            Ok(response) => {
                let max_seq = response
                    .items
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|obj| {
                        // Extract end sequence from batch filename
                        let filename = obj.name.rsplit('/').next()?;
                        let (_, end) = Self::parse_batch_filename(filename)?;
                        Some(end)
                    })
                    .max();
                Ok(max_seq)
            }
            Err(e) => Err(PublishError::Storage(format!(
                "Failed to list batch objects: {e}"
            ))),
        }
    }

    /// Find the batch containing a specific sequence number
    async fn find_batch_containing(&self, sequence: u64) -> Result<Option<String>, PublishError> {
        use google_cloud_storage::http::objects::list::ListObjectsRequest;

        let prefix = format!("{}/batches/", self.config.prefix);
        let request = ListObjectsRequest {
            bucket: self.config.bucket.clone(),
            prefix: Some(prefix),
            ..Default::default()
        };

        match self.client.list_objects(&request).await {
            Ok(response) => {
                for obj in response.items.unwrap_or_default() {
                    let filename = match obj.name.rsplit('/').next() {
                        Some(f) => f,
                        None => continue,
                    };
                    if let Some((start, end)) = Self::parse_batch_filename(filename) {
                        if sequence >= start && sequence <= end {
                            return Ok(Some(obj.name));
                        }
                    }
                }
                Ok(None)
            }
            Err(e) => Err(PublishError::Storage(format!(
                "Failed to list batch objects: {e}"
            ))),
        }
    }
}

#[async_trait]
impl DAPublisher for GcsPublisher {
    fn name(&self) -> &str {
        "gcs"
    }

    async fn publish(&self, message: &SignedMessage) -> PublishResult {
        // Wrap single message in a batch
        let batch = SignedBatch {
            start_sequence: message.sequence,
            end_sequence: message.sequence,
            messages: vec![message.clone()],
            batch_signature: message.signature.clone(),
            signer: message.signer.clone(),
            created_at: message.timestamp,
        };
        self.publish_batch(&batch).await
    }

    async fn publish_batch(&self, batch: &SignedBatch) -> PublishResult {
        let path = self.batch_path(batch.start_sequence, batch.end_sequence);

        // Serialize batch
        let data = match serde_json::to_vec_pretty(batch) {
            Ok(d) => d,
            Err(e) => {
                error!(
                    start = batch.start_sequence,
                    end = batch.end_sequence,
                    error = %e,
                    "Failed to serialize batch"
                );
                return PublishResult::failure("gcs", format!("Serialization error: {e}"));
            }
        };

        match self.upload(&path, data).await {
            Ok(()) => {
                info!(
                    start = batch.start_sequence,
                    end = batch.end_sequence,
                    messages = batch.messages.len(),
                    path = %path,
                    "Batch published to GCS"
                );
                let reference = format!("gs://{}/{}", self.config.bucket, path);
                PublishResult::success("gcs", reference)
            }
            Err(e) => {
                error!(
                    start = batch.start_sequence,
                    end = batch.end_sequence,
                    error = %e,
                    "Failed to upload batch to GCS"
                );
                PublishResult::failure("gcs", format!("{e}"))
            }
        }
    }

    async fn get(&self, sequence: u64) -> Result<Option<SignedMessage>, PublishError> {
        // Find the batch containing this sequence
        if let Some(batch_path) = self.find_batch_containing(sequence).await? {
            if let Some(data) = self.download(&batch_path).await? {
                let batch: SignedBatch = serde_json::from_slice(&data).map_err(|e| {
                    PublishError::Serialization(format!("Failed to parse batch: {e}"))
                })?;
                return Ok(batch.messages.into_iter().find(|m| m.sequence == sequence));
            }
        }
        Ok(None)
    }

    async fn get_batch(&self, start_sequence: u64) -> Result<Option<SignedBatch>, PublishError> {
        use google_cloud_storage::http::objects::list::ListObjectsRequest;

        // Find batch that starts with this sequence
        let prefix = format!("{}/batches/{:012}_", self.config.prefix, start_sequence);
        let request = ListObjectsRequest {
            bucket: self.config.bucket.clone(),
            prefix: Some(prefix),
            ..Default::default()
        };

        match self.client.list_objects(&request).await {
            Ok(response) => {
                if let Some(obj) = response.items.and_then(|items| items.into_iter().next()) {
                    if let Some(data) = self.download(&obj.name).await? {
                        let batch: SignedBatch = serde_json::from_slice(&data).map_err(|e| {
                            PublishError::Serialization(format!("Failed to parse batch: {e}"))
                        })?;
                        return Ok(Some(batch));
                    }
                }
                Ok(None)
            }
            Err(e) => Err(PublishError::Storage(format!(
                "Failed to list batch objects: {e}"
            ))),
        }
    }

    async fn get_latest_sequence(&self) -> Result<Option<u64>, PublishError> {
        self.get_latest_batch_sequence().await
    }

    async fn save_state(&self, _sequence: u64) -> Result<(), PublishError> {
        // State is implicit in batch filenames - no separate state file needed
        Ok(())
    }

    async fn load_state(&self) -> Result<Option<u64>, PublishError> {
        // State is implicit in batch filenames
        let seq = self.get_latest_batch_sequence().await?;
        if let Some(s) = seq {
            info!(sequence = s, "Loaded state from batch files");
        } else {
            warn!("No existing batches found in GCS, starting fresh");
        }
        Ok(seq)
    }
}

// Stub implementation when GCS feature is not enabled
#[cfg(not(feature = "gcs"))]
#[derive(Debug)]
pub struct GcsPublisher {
    _config: GcsConfig,
}

#[cfg(not(feature = "gcs"))]
impl GcsPublisher {
    pub async fn new(_config: GcsConfig) -> Result<Self, PublishError> {
        Err(PublishError::Config(
            "GCS feature not enabled. Compile with --features gcs".to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gcs_config_defaults() {
        let config = GcsConfig::new("my-bucket");
        assert_eq!(config.bucket, "my-bucket");
        assert_eq!(config.prefix, "sequencer");
    }

    #[test]
    fn test_gcs_config_with_prefix() {
        let config = GcsConfig::new("my-bucket").with_prefix("custom/path");
        assert_eq!(config.prefix, "custom/path");
    }

    #[test]
    fn test_parse_batch_filename_valid() {
        // Standard batch filename
        let result = GcsPublisher::parse_batch_filename("000000000001_000000000050.json");
        assert_eq!(result, Some((1, 50)));

        // Large sequence numbers
        let result = GcsPublisher::parse_batch_filename("000000001000_000000002000.json");
        assert_eq!(result, Some((1000, 2000)));

        // Single message batch
        let result = GcsPublisher::parse_batch_filename("000000000042_000000000042.json");
        assert_eq!(result, Some((42, 42)));
    }

    #[test]
    fn test_parse_batch_filename_invalid() {
        // Missing .json extension
        assert_eq!(
            GcsPublisher::parse_batch_filename("000000000001_000000000050"),
            None
        );

        // Wrong extension
        assert_eq!(
            GcsPublisher::parse_batch_filename("000000000001_000000000050.txt"),
            None
        );

        // Missing underscore
        assert_eq!(
            GcsPublisher::parse_batch_filename("000000000001000000000050.json"),
            None
        );

        // Extra underscore
        assert_eq!(
            GcsPublisher::parse_batch_filename("000000000001_000000000050_extra.json"),
            None
        );

        // Non-numeric
        assert_eq!(
            GcsPublisher::parse_batch_filename("abcdef_ghijkl.json"),
            None
        );

        // Empty
        assert_eq!(GcsPublisher::parse_batch_filename(""), None);
    }

    #[test]
    fn test_batch_filename_sorting() {
        // Verify that batch filenames sort correctly lexicographically
        let mut filenames = vec![
            "000000000051_000000000100.json",
            "000000000001_000000000050.json",
            "000000000101_000000000150.json",
        ];
        filenames.sort();

        assert_eq!(
            filenames,
            vec![
                "000000000001_000000000050.json",
                "000000000051_000000000100.json",
                "000000000101_000000000150.json",
            ]
        );
    }
}
