//! Google Cloud Storage fetcher implementation
//!
//! Fetches signed batches from GCS with the same path structure used by the sequencer:
//! ```text
//! gs://{bucket}/{prefix}/
//! └── batches/
//!     ├── 000000000001_000000000050.cbor.zst
//!     ├── 000000000051_000000000100.cbor.zst
//!     └── ...
//! ```
//!
//! Batch filenames follow the pattern `{start:012}_{end:012}.cbor.zst` where:
//! - `start` is the first sequence number in the batch (inclusive)
//! - `end` is the last sequence number in the batch (inclusive)
//! - Both are zero-padded to 12 digits
//! - Format is CBOR with zstd compression
//!
//! # Performance Note
//!
//! The `get()` method for single messages is inefficient as it lists all batches
//! to find the containing batch. Use batch sync mode (via `BatchIndex`) for
//! efficient sequential fetching. The validator's `run_batched()` handles this
//! automatically.

use crate::sync::fetcher::{BatchInfo, StorageFetcher};
use anyhow::{Context, Result};
use async_trait::async_trait;
use synddb_shared::{
    gcs::GcsConfig,
    types::{
        batch::parse_batch_filename,
        cbor::batch::CborBatch,
        message::{SignedBatch, SignedMessage},
    },
};
use tracing::{debug, info, warn};

/// Google Cloud Storage fetcher
pub struct GcsFetcher {
    client: google_cloud_storage::client::Client,
    config: GcsConfig,
}

impl std::fmt::Debug for GcsFetcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GcsFetcher")
            .field("bucket", &self.config.bucket)
            .field("prefix", &self.config.prefix)
            .field("emulator", &self.config.is_emulator())
            .finish()
    }
}

impl GcsFetcher {
    /// Create a new GCS fetcher from config
    ///
    /// Uses default credentials (`GOOGLE_APPLICATION_CREDENTIALS` env var,
    /// workload identity, or metadata server).
    ///
    /// If `emulator_host` is set in config, uses anonymous authentication and
    /// connects to the specified emulator instead of real GCS.
    pub async fn from_config(config: GcsConfig) -> Result<Self> {
        use google_cloud_storage::client::{Client, ClientConfig};

        let client_config = if let Some(ref emulator_host) = config.emulator_host {
            info!(emulator_host = %emulator_host, "Using GCS emulator");
            let mut cfg = ClientConfig::default().anonymous();
            cfg.storage_endpoint = emulator_host.clone();
            cfg
        } else {
            ClientConfig::default()
                .with_auth()
                .await
                .context("Failed to configure GCS auth")?
        };

        let client = Client::new(client_config);

        info!(bucket = %config.bucket, prefix = %config.prefix, "GCS fetcher initialized");

        Ok(Self { client, config })
    }

    /// Create a new GCS fetcher (convenience constructor)
    ///
    /// Prefer `from_config()` for more control. This method is kept for
    /// backwards compatibility.
    pub async fn new(
        bucket: String,
        prefix: String,
        emulator_host: Option<String>,
    ) -> Result<Self> {
        let mut config = GcsConfig::new(bucket).with_prefix(prefix);
        if let Some(host) = emulator_host {
            config = config.with_emulator_host(host);
        }
        Self::from_config(config).await
    }

    /// Download data from GCS
    async fn download(&self, path: &str) -> Result<Option<Vec<u8>>> {
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

        let prefix = format!("{}/batches/", self.config.prefix);
        let request = ListObjectsRequest {
            bucket: self.config.bucket.clone(),
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
                        let (start, end) = parse_batch_filename(filename)?;
                        debug!(filename, start, end, "Parsed batch file");
                        Some(BatchInfo::new(start, end, obj.name.clone()))
                    })
                    .collect();

                // Sort by start sequence
                batches.sort_by_key(|b| b.start_sequence);

                info!(count = batches.len(), "Listed batches from GCS");
                Ok(batches)
            }
            Err(e) => Err(anyhow::anyhow!("Failed to list batch objects: {e}")),
        }
    }

    async fn get_batch(&self, start_sequence: u64) -> Result<Option<SignedBatch>> {
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
                info!(path, bytes = data.len(), "Parsing CBOR batch");

                // Parse CBOR + zstd format
                let cbor_batch = CborBatch::from_cbor_zstd(&data)
                    .with_context(|| format!("Failed to decompress/parse CBOR batch at {path}"))?;

                info!(
                    start = cbor_batch.start_sequence,
                    end = cbor_batch.end_sequence,
                    messages = cbor_batch.messages.len(),
                    content_hash = %cbor_batch.content_hash_hex(),
                    "Parsed CBOR batch, converting to SignedBatch"
                );

                // Convert to SignedBatch for unified processing
                let batch = cbor_batch
                    .to_signed_batch()
                    .with_context(|| format!("Failed to convert CBOR batch at {path}"))?;

                info!(
                    start = batch.start_sequence,
                    end = batch.end_sequence,
                    messages = batch.messages.len(),
                    signer = %batch.signer,
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
    use synddb_shared::types::batch::format_batch_filename;

    #[test]
    fn test_batch_path_format() {
        let prefix = "sequencer";
        let path = format!("{}/batches/{}", prefix, format_batch_filename(1, 50));
        assert_eq!(path, "sequencer/batches/000000000001_000000000050.cbor.zst");

        let path = format!("{}/batches/{}", prefix, format_batch_filename(0, 0));
        assert_eq!(path, "sequencer/batches/000000000000_000000000000.cbor.zst");

        let path = format!(
            "{}/batches/{}",
            prefix,
            format_batch_filename(999_999_999_999, 999_999_999_999)
        );
        assert_eq!(path, "sequencer/batches/999999999999_999999999999.cbor.zst");
    }
}

// Additional tests for parse_batch_filename are in synddb-shared::types::batch
