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
use google_cloud_storage::client::{Storage, StorageControl};
use synddb_shared::{
    gcs::GcsConfig,
    types::{
        batch::parse_batch_filename,
        cbor::batch::CborBatch,
        message::{SignedBatch, SignedMessage},
    },
};
use tracing::{debug, info};

/// Google Cloud Storage fetcher
pub struct GcsFetcher {
    storage: Storage,
    control: StorageControl,
    config: GcsConfig,
    /// HTTP client for emulator REST API (`StorageControl` uses gRPC which fake-gcs-server doesn't support)
    emulator_client: Option<reqwest::Client>,
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
    /// If `emulator_host` is set in config, uses the emulator instead of real GCS.
    pub async fn from_config(config: GcsConfig) -> Result<Self> {
        let (storage, control, emulator_client) =
            if let Some(ref emulator_host) = config.emulator_host {
                info!(emulator_host = %emulator_host, "Using GCS emulator");

                // For emulator mode, we need:
                // 1. Custom endpoint pointing to the emulator
                // 2. Anonymous credentials (no authentication)
                // 3. HTTP client for REST API (StorageControl uses gRPC which fake-gcs-server doesn't support)
                use google_cloud_auth::credentials::anonymous;
                let anonymous_creds = anonymous::Builder::default().build();

                let storage = Storage::builder()
                    .with_endpoint(emulator_host)
                    .with_credentials(anonymous_creds.clone())
                    .build()
                    .await
                    .context("Failed to create Storage client for emulator")?;

                let control = StorageControl::builder()
                    .with_endpoint(emulator_host)
                    .with_credentials(anonymous_creds)
                    .build()
                    .await
                    .context("Failed to create StorageControl client for emulator")?;

                let http_client = reqwest::Client::new();

                (storage, control, Some(http_client))
            } else {
                let storage = Storage::builder()
                    .build()
                    .await
                    .context("Failed to create Storage client")?;

                let control = StorageControl::builder()
                    .build()
                    .await
                    .context("Failed to create StorageControl client")?;

                (storage, control, None)
            };

        info!(bucket = %config.bucket, prefix = %config.prefix, "GCS fetcher initialized");

        Ok(Self {
            storage,
            control,
            config,
            emulator_client,
        })
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

    /// Get the bucket path in the format required by the Google Cloud Storage SDK
    fn bucket_path(&self) -> String {
        format!("projects/_/buckets/{}", self.config.bucket)
    }

    /// Download data from GCS
    async fn download(&self, path: &str) -> Result<Option<Vec<u8>>> {
        let mut reader = match self
            .storage
            .read_object(self.bucket_path(), path)
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
                }
                return Err(anyhow::anyhow!("Failed to download from GCS: {e}"));
            }
        };

        // Read all chunks into a buffer
        let mut data = Vec::new();
        while let Some(chunk) = reader.next().await {
            let chunk = chunk.map_err(|e| anyhow::anyhow!("Failed to read chunk from GCS: {e}"))?;
            data.extend_from_slice(&chunk);
        }

        Ok(Some(data))
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

    /// List batches using the GCS REST API (for emulator mode)
    async fn list_batches_emulator(&self, client: &reqwest::Client) -> Result<Vec<BatchInfo>> {
        let emulator_host = self
            .config
            .emulator_host
            .as_ref()
            .expect("emulator_client exists only when emulator_host is set");

        let prefix = format!("{}/batches/", self.config.prefix);
        let mut batches = Vec::new();
        let mut page_token: Option<String> = None;

        loop {
            use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
            let encoded_prefix = utf8_percent_encode(&prefix, NON_ALPHANUMERIC).to_string();
            let mut url = format!(
                "{}/storage/v1/b/{}/o?prefix={}",
                emulator_host, self.config.bucket, encoded_prefix
            );
            if let Some(ref token) = page_token {
                let encoded_token = utf8_percent_encode(token, NON_ALPHANUMERIC).to_string();
                url.push_str(&format!("&pageToken={}", encoded_token));
            }

            let response = client
                .get(&url)
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("Failed to list objects from emulator: {e}"))?;

            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                return Err(anyhow::anyhow!(
                    "Emulator list objects failed: {} - {}",
                    status,
                    body
                ));
            }

            let body: serde_json::Value = response
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("Failed to parse emulator response: {e}"))?;

            // Parse items from response
            if let Some(items) = body.get("items").and_then(|v| v.as_array()) {
                for item in items {
                    if let Some(name) = item.get("name").and_then(|v| v.as_str()) {
                        if let Some(filename) = name.rsplit('/').next() {
                            if let Some((start, end)) = parse_batch_filename(filename) {
                                debug!(filename, start, end, "Parsed batch file (emulator)");
                                batches.push(BatchInfo::new(start, end, name.to_string()));
                            }
                        }
                    }
                }
            }

            // Check for next page
            match body.get("nextPageToken").and_then(|v| v.as_str()) {
                Some(token) if !token.is_empty() => {
                    debug!(token = %token, "Fetching next page of batches (emulator)");
                    page_token = Some(token.to_string());
                }
                _ => break,
            }
        }

        batches.sort_by_key(|b| b.start_sequence);
        info!(count = batches.len(), "Listed batches from GCS (emulator)");
        Ok(batches)
    }

    /// Find a batch by prefix using the REST API (for emulator mode)
    async fn find_batch_by_prefix_emulator(
        &self,
        client: &reqwest::Client,
        prefix: &str,
    ) -> Result<Option<String>> {
        let emulator_host = self
            .config
            .emulator_host
            .as_ref()
            .expect("emulator_client exists only when emulator_host is set");

        use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
        let encoded_prefix = utf8_percent_encode(prefix, NON_ALPHANUMERIC).to_string();
        let url = format!(
            "{}/storage/v1/b/{}/o?prefix={}&maxResults=1",
            emulator_host, self.config.bucket, encoded_prefix
        );

        let response = client
            .get(&url)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to list objects from emulator: {e}"))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!(
                "Emulator list objects failed: {} - {}",
                status,
                body
            ));
        }

        let body: serde_json::Value = response
            .json()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to parse emulator response: {e}"))?;

        Ok(body
            .get("items")
            .and_then(|v| v.as_array())
            .and_then(|items| items.first())
            .and_then(|item| item.get("name"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()))
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
            debug!("No batches found in GCS");
        }

        Ok(max_seq)
    }

    async fn list_batches(&self) -> Result<Vec<BatchInfo>> {
        // Use REST API for emulator mode (StorageControl uses gRPC which fake-gcs-server doesn't support)
        if let Some(ref client) = self.emulator_client {
            return self.list_batches_emulator(client).await;
        }

        let prefix = format!("{}/batches/", self.config.prefix);
        let mut batches = Vec::new();
        let mut page_token: Option<String> = None;

        loop {
            let mut request = self
                .control
                .list_objects()
                .set_parent(self.bucket_path())
                .set_prefix(&prefix);

            if let Some(ref token) = page_token {
                request = request.set_page_token(token);
            }

            let response = request
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("Failed to list batch objects: {e}"))?;

            // Parse batch info from object names
            for obj in response.objects {
                if let Some(filename) = obj.name.rsplit('/').next() {
                    if let Some((start, end)) = parse_batch_filename(filename) {
                        debug!(filename, start, end, "Parsed batch file");
                        batches.push(BatchInfo::new(start, end, obj.name.clone()));
                    }
                }
            }

            // Check for more pages
            if response.next_page_token.is_empty() {
                break;
            }
            debug!(token = %response.next_page_token, "Fetching next page of batches");
            page_token = Some(response.next_page_token);
        }

        // Sort by start sequence
        batches.sort_by_key(|b| b.start_sequence);

        info!(count = batches.len(), "Listed batches from GCS");
        Ok(batches)
    }

    async fn get_batch(&self, start_sequence: u64) -> Result<Option<SignedBatch>> {
        // Find batch that starts with this sequence by listing with a specific prefix
        let prefix = format!("{}/batches/{:012}_", self.config.prefix, start_sequence);

        // Use REST API for emulator mode
        let obj_name = if let Some(ref client) = self.emulator_client {
            self.find_batch_by_prefix_emulator(client, &prefix).await?
        } else {
            let response = self
                .control
                .list_objects()
                .set_parent(self.bucket_path())
                .set_prefix(&prefix)
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("Failed to list batch objects: {e}"))?;

            response.objects.into_iter().next().map(|obj| obj.name)
        };

        if let Some(name) = obj_name {
            return self.get_batch_by_path(&name).await;
        }

        Ok(None)
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
