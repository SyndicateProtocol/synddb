//! Google Cloud Storage fetcher implementation
//!
//! Fetches signed messages from GCS with the same path structure used by the sequencer:
//! ```text
//! gs://{bucket}/{prefix}/
//! ├── messages/
//! │   ├── 000000000001.json
//! │   ├── 000000000002.json
//! │   └── ...
//! └── state/
//!     └── sequence.json
//! ```

use crate::sync::fetcher::DAFetcher;
use anyhow::{Context, Result};
use async_trait::async_trait;
use synddb_shared::types::message::SignedMessage;
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
    pub async fn new(bucket: String, prefix: String) -> Result<Self> {
        use google_cloud_storage::client::{Client, ClientConfig};

        let client_config = ClientConfig::default()
            .with_auth()
            .await
            .context("Failed to configure GCS auth")?;

        let client = Client::new(client_config);

        info!(bucket = %bucket, prefix = %prefix, "GCS fetcher initialized");

        Ok(Self {
            client,
            bucket,
            prefix,
        })
    }

    /// Get the full path for a message (must match sequencer format exactly)
    fn message_path(&self, sequence: u64) -> String {
        format!("{}/messages/{:012}.json", self.prefix, sequence)
    }
}

#[async_trait]
impl DAFetcher for GcsFetcher {
    fn name(&self) -> &str {
        "gcs"
    }

    async fn get(&self, sequence: u64) -> Result<Option<SignedMessage>> {
        use google_cloud_storage::http::objects::download::Range;
        use google_cloud_storage::http::objects::get::GetObjectRequest;

        let path = self.message_path(sequence);
        let request = GetObjectRequest {
            bucket: self.bucket.clone(),
            object: path.clone(),
            ..Default::default()
        };

        match self
            .client
            .download_object(&request, &Range::default())
            .await
        {
            Ok(data) => {
                let message: SignedMessage = serde_json::from_slice(&data)
                    .with_context(|| format!("Failed to parse message at {path}"))?;
                debug!(sequence, "Fetched message from GCS");
                Ok(Some(message))
            }
            Err(e) => {
                let error_str = e.to_string();
                if error_str.contains("404") || error_str.contains("No such object") {
                    debug!(sequence, "Message not found in GCS");
                    Ok(None)
                } else {
                    Err(anyhow::anyhow!("Failed to download from GCS: {e}"))
                }
            }
        }
    }

    async fn get_latest_sequence(&self) -> Result<Option<u64>> {
        use google_cloud_storage::http::objects::list::ListObjectsRequest;

        let prefix = format!("{}/messages/", self.prefix);
        let request = ListObjectsRequest {
            bucket: self.bucket.clone(),
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
                        // Extract sequence from path like "prefix/messages/000000000042.json"
                        let name: &str = &obj.name;
                        name.rsplit('/')
                            .next()
                            .and_then(|filename| filename.strip_suffix(".json"))
                            .and_then(|seq_str| seq_str.parse::<u64>().ok())
                    })
                    .max();

                if let Some(seq) = max_seq {
                    debug!(sequence = seq, "Found latest sequence in GCS");
                } else {
                    warn!("No messages found in GCS");
                }

                Ok(max_seq)
            }
            Err(e) => Err(anyhow::anyhow!("Failed to list objects: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_message_path_format() {
        // We can't create a real GcsFetcher without GCS, but we can verify the path format
        let prefix = "sequencer";
        let path = format!("{}/messages/{:012}.json", prefix, 42);
        assert_eq!(path, "sequencer/messages/000000000042.json");

        let path = format!("{}/messages/{:012}.json", prefix, 0);
        assert_eq!(path, "sequencer/messages/000000000000.json");

        let path = format!("{}/messages/{:012}.json", prefix, 999_999_999_999_u64);
        assert_eq!(path, "sequencer/messages/999999999999.json");
    }
}
