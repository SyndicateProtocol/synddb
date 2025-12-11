//! Google Cloud Storage fetcher implementation
//!
//! Fetches signed batches from GCS with the same path structure used by the sequencer:
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
//! - Both are zero-padded to 12 digits
//!
//! TODO revisit this
//! # Performance Note
//!
//! The `get()` method for single messages is inefficient as it lists all batches
//! to find the containing batch. Use batch sync mode (via `BatchIndex`) for
//! efficient sequential fetching. The validator's `run_batched()` handles this
//! automatically.

use crate::sync::fetcher::{BatchInfo, StorageFetcher};
use anyhow::{Context, Result};
use async_trait::async_trait;
use synddb_shared::types::message::{SignedBatch, SignedMessage};
use tracing::{debug, info, warn};

/// Google Cloud Storage fetcher
pub struct GcsFetcher {
    client: google_cloud_storage::client::Client,
    bucket: String,
    prefix: String,
}

impl std::fmt::Debug for GcsFetcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GcsFetcher")
            .field("bucket", &self.bucket)
            .field("prefix", &self.prefix)
            .finish()
    }
}

impl GcsFetcher {
    /// Create a new GCS fetcher
    ///
    /// Uses default credentials (`GOOGLE_APPLICATION_CREDENTIALS` env var,
    /// workload identity, or metadata server).
    ///
    /// If `emulator_host` is provided, uses anonymous authentication and
    /// connects to the specified emulator instead of real GCS.
    pub async fn new(
        bucket: String,
        prefix: String,
        emulator_host: Option<String>,
    ) -> Result<Self> {
        use google_cloud_storage::client::{Client, ClientConfig};

        // Normalize emulator_host: treat empty strings as None
        let emulator_host = emulator_host.filter(|s| !s.is_empty());

        let client_config = if let Some(ref emulator_host) = emulator_host {
            // Emulator mode: use anonymous auth and custom endpoint
            info!(emulator_host = %emulator_host, "Using GCS emulator");
            let mut cfg = ClientConfig::default().anonymous();
            cfg.storage_endpoint = emulator_host.clone();
            cfg
        } else {
            // Production mode: use real GCS with authentication
            ClientConfig::default()
                .with_auth()
                .await
                .context("Failed to configure GCS auth")?
        };

        let client = Client::new(client_config);

        info!(bucket = %bucket, prefix = %prefix, "GCS fetcher initialized");

        Ok(Self {
            client,
            bucket,
            prefix,
        })
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

    /// Download data from GCS
    async fn download(&self, path: &str) -> Result<Option<Vec<u8>>> {
        use google_cloud_storage::http::objects::download::Range;
        use google_cloud_storage::http::objects::get::GetObjectRequest;

        let request = GetObjectRequest {
            bucket: self.bucket.clone(),
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
                    Err(anyhow::anyhow!("Failed to download from GCS: {e}"))
                }
            }
        }
    }

    /// Find the batch containing a specific sequence number
    async fn find_batch_containing(&self, sequence: u64) -> Result<Option<SignedBatch>> {
        let batches = self.list_batches().await?;
        for info in batches {
            if info.contains(sequence) {
                return self.get_batch_by_path(&info.path).await;
            }
        }
        Ok(None)
    }
}

#[async_trait]
impl StorageFetcher for GcsFetcher {
    fn name(&self) -> &str {
        "gcs"
    }

    fn supports_batches(&self) -> bool {
        true
    }

    async fn get(&self, sequence: u64) -> Result<Option<SignedMessage>> {
        // Find the batch containing this sequence and extract the message
        match self.find_batch_containing(sequence).await? {
            Some(batch) => {
                let msg = batch.messages.into_iter().find(|m| m.sequence == sequence);
                if msg.is_some() {
                    debug!(sequence, "Fetched message from GCS batch");
                } else {
                    debug!(sequence, "Message not found in batch");
                }
                Ok(msg)
            }
            None => {
                debug!(sequence, "No batch containing sequence found in GCS");
                Ok(None)
            }
        }
    }

    async fn get_latest_sequence(&self) -> Result<Option<u64>> {
        let batches = self.list_batches().await?;
        let max_seq = batches.iter().map(|b| b.end_sequence).max();

        if let Some(seq) = max_seq {
            debug!(sequence = seq, "Found latest sequence in GCS");
        } else {
            warn!("No batches found in GCS");
        }

        Ok(max_seq)
    }

    //TODO: GCS returns max 1000 objects per request. This implementation does not
    // handle pagination, so batches beyond the first 1000 will be silently missed.
    // At scale (~50k+ messages), consider either:
    // 1. Implementing pagination via `next_page_token`
    // 2. A smarter sync strategy that checkpoints batch boundaries locally
    async fn list_batches(&self) -> Result<Vec<BatchInfo>> {
        use google_cloud_storage::http::objects::list::ListObjectsRequest;

        let prefix = format!("{}/batches/", self.prefix);
        let request = ListObjectsRequest {
            bucket: self.bucket.clone(),
            prefix: Some(prefix),
            ..Default::default()
        };

        match self.client.list_objects(&request).await {
            Ok(response) => {
                let mut batches: Vec<BatchInfo> = response
                    .items
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|obj| {
                        let filename = obj.name.rsplit('/').next()?;
                        let (start, end) = Self::parse_batch_filename(filename)?;
                        Some(BatchInfo::new(start, end, obj.name.clone()))
                    })
                    .collect();

                // Sort by start sequence
                batches.sort_by_key(|b| b.start_sequence);

                debug!(count = batches.len(), "Listed batches from GCS");
                Ok(batches)
            }
            Err(e) => Err(anyhow::anyhow!("Failed to list batch objects: {e}")),
        }
    }

    async fn get_batch(&self, start_sequence: u64) -> Result<Option<SignedBatch>> {
        use google_cloud_storage::http::objects::list::ListObjectsRequest;

        // Find batch that starts with this sequence
        let prefix = format!("{}/batches/{:012}_", self.prefix, start_sequence);
        let request = ListObjectsRequest {
            bucket: self.bucket.clone(),
            prefix: Some(prefix),
            ..Default::default()
        };

        match self.client.list_objects(&request).await {
            Ok(response) => {
                if let Some(obj) = response.items.and_then(|items| items.into_iter().next()) {
                    return self.get_batch_by_path(&obj.name).await;
                }
                Ok(None)
            }
            Err(e) => Err(anyhow::anyhow!("Failed to list batch objects: {e}")),
        }
    }

    async fn get_batch_by_path(&self, path: &str) -> Result<Option<SignedBatch>> {
        match self.download(path).await? {
            Some(data) => {
                let batch: SignedBatch = serde_json::from_slice(&data)
                    .with_context(|| format!("Failed to parse batch at {path}"))?;
                debug!(
                    start = batch.start_sequence,
                    end = batch.end_sequence,
                    messages = batch.messages.len(),
                    "Fetched batch from GCS"
                );
                Ok(Some(batch))
            }
            None => {
                debug!(path, "Batch not found in GCS");
                Ok(None)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_batch_path_format() {
        let prefix = "sequencer";
        let path = format!("{}/batches/{:012}_{:012}.json", prefix, 1, 50);
        assert_eq!(path, "sequencer/batches/000000000001_000000000050.json");

        let path = format!("{}/batches/{:012}_{:012}.json", prefix, 0, 0);
        assert_eq!(path, "sequencer/batches/000000000000_000000000000.json");

        let path = format!(
            "{}/batches/{:012}_{:012}.json",
            prefix, 999_999_999_999_u64, 999_999_999_999_u64
        );
        assert_eq!(path, "sequencer/batches/999999999999_999999999999.json");
    }

    #[test]
    fn test_parse_batch_filename_valid() {
        let result = GcsFetcher::parse_batch_filename("000000000001_000000000050.json");
        assert_eq!(result, Some((1, 50)));

        let result = GcsFetcher::parse_batch_filename("000000001000_000000002000.json");
        assert_eq!(result, Some((1000, 2000)));

        // Single message batch
        let result = GcsFetcher::parse_batch_filename("000000000042_000000000042.json");
        assert_eq!(result, Some((42, 42)));
    }

    #[test]
    fn test_parse_batch_filename_invalid() {
        // Missing .json extension
        assert_eq!(
            GcsFetcher::parse_batch_filename("000000000001_000000000050"),
            None
        );

        // Wrong extension
        assert_eq!(
            GcsFetcher::parse_batch_filename("000000000001_000000000050.txt"),
            None
        );

        // Missing underscore
        assert_eq!(
            GcsFetcher::parse_batch_filename("000000000001000000000050.json"),
            None
        );

        // Extra underscore
        assert_eq!(
            GcsFetcher::parse_batch_filename("000000000001_000000000050_extra.json"),
            None
        );

        // Non-numeric
        assert_eq!(GcsFetcher::parse_batch_filename("abcdef_ghijkl.json"), None);

        // Empty
        assert_eq!(GcsFetcher::parse_batch_filename(""), None);
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
