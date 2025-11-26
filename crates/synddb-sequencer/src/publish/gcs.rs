//! Google Cloud Storage publisher implementation
//!
//! Stores signed messages in GCS with the following structure:
//! ```text
//! gs://{bucket}/{prefix}/
//! ├── messages/
//! │   ├── 000000000001.json
//! │   ├── 000000000002.json
//! │   └── ...
//! └── state/
//!     └── sequence.json
//! ```

use super::PublishError;
use serde::{Deserialize, Serialize};

use {
    super::{DAPublisher, PublishResult},
    crate::inbox::SignedMessage,
    async_trait::async_trait,
    tracing::{debug, error, info, warn},
};

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

/// State persisted for recovery
#[derive(Debug, Serialize, Deserialize)]
struct SequencerState {
    /// Last successfully published sequence number
    last_sequence: u64,
    /// Timestamp when state was saved
    updated_at: u64,
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

    /// Get the full path for a message
    fn message_path(&self, sequence: u64) -> String {
        format!("{}/messages/{:012}.json", self.config.prefix, sequence)
    }

    /// Get the path for state file
    fn state_path(&self) -> String {
        format!("{}/state/sequence.json", self.config.prefix)
    }
}

#[async_trait]
impl DAPublisher for GcsPublisher {
    fn name(&self) -> &str {
        "gcs"
    }

    async fn publish(&self, message: &SignedMessage) -> PublishResult {
        let path = self.message_path(message.sequence);

        // Serialize message
        let data = match serde_json::to_vec_pretty(message) {
            Ok(d) => d,
            Err(e) => {
                error!(sequence = message.sequence, error = %e, "Failed to serialize message");
                return PublishResult::failure("gcs", format!("Serialization error: {e}"));
            }
        };

        // Upload to GCS
        use google_cloud_storage::http::objects::upload::{Media, UploadObjectRequest, UploadType};

        let upload_type = UploadType::Simple(Media::new(path.clone()));
        let request = UploadObjectRequest {
            bucket: self.config.bucket.clone(),
            ..Default::default()
        };

        match self
            .client
            .upload_object(&request, data, &upload_type)
            .await
        {
            Ok(_) => {
                debug!(sequence = message.sequence, path = %path, "Message published to GCS");
                let reference = format!("gs://{}/{}", self.config.bucket, path);
                PublishResult::success("gcs", reference)
            }
            Err(e) => {
                error!(sequence = message.sequence, error = %e, "Failed to upload to GCS");
                PublishResult::failure("gcs", format!("Upload error: {e}"))
            }
        }
    }

    async fn get(&self, sequence: u64) -> Result<Option<SignedMessage>, PublishError> {
        let path = self.message_path(sequence);

        use google_cloud_storage::http::objects::download::Range;
        use google_cloud_storage::http::objects::get::GetObjectRequest;

        let request = GetObjectRequest {
            bucket: self.config.bucket.clone(),
            object: path.clone(),
            ..Default::default()
        };

        match self
            .client
            .download_object(&request, &Range::default())
            .await
        {
            Ok(data) => {
                let message: SignedMessage = serde_json::from_slice(&data).map_err(|e| {
                    PublishError::Serialization(format!("Failed to parse message: {e}"))
                })?;
                Ok(Some(message))
            }
            Err(e) => {
                // Check if it's a 404
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

    async fn get_latest_sequence(&self) -> Result<Option<u64>, PublishError> {
        // List objects in messages/ directory, get the highest sequence
        use google_cloud_storage::http::objects::list::ListObjectsRequest;

        let prefix = format!("{}/messages/", self.config.prefix);
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
                        // Extract sequence from path like "prefix/messages/000000000042.json"
                        // obj.name is a String
                        let name: &str = &obj.name;
                        name.rsplit('/')
                            .next()
                            .and_then(|filename| filename.strip_suffix(".json"))
                            .and_then(|seq_str| seq_str.parse::<u64>().ok())
                    })
                    .max();
                Ok(max_seq)
            }
            Err(e) => Err(PublishError::Storage(format!(
                "Failed to list objects: {e}"
            ))),
        }
    }

    async fn save_state(&self, sequence: u64) -> Result<(), PublishError> {
        let path = self.state_path();

        let state = SequencerState {
            last_sequence: sequence,
            updated_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        };

        let data = serde_json::to_vec_pretty(&state)
            .map_err(|e| PublishError::Serialization(format!("Failed to serialize state: {e}")))?;

        use google_cloud_storage::http::objects::upload::{Media, UploadObjectRequest, UploadType};

        let upload_type = UploadType::Simple(Media::new(path.clone()));
        let request = UploadObjectRequest {
            bucket: self.config.bucket.clone(),
            ..Default::default()
        };

        self.client
            .upload_object(&request, data, &upload_type)
            .await
            .map_err(|e| PublishError::Storage(format!("Failed to save state: {e}")))?;

        debug!(sequence, "State saved to GCS");
        Ok(())
    }

    async fn load_state(&self) -> Result<Option<u64>, PublishError> {
        let path = self.state_path();

        use google_cloud_storage::http::objects::download::Range;
        use google_cloud_storage::http::objects::get::GetObjectRequest;

        let request = GetObjectRequest {
            bucket: self.config.bucket.clone(),
            object: path,
            ..Default::default()
        };

        match self
            .client
            .download_object(&request, &Range::default())
            .await
        {
            Ok(data) => {
                let state: SequencerState = serde_json::from_slice(&data).map_err(|e| {
                    PublishError::Serialization(format!("Failed to parse state: {e}"))
                })?;
                info!(sequence = state.last_sequence, "Loaded state from GCS");
                Ok(Some(state.last_sequence))
            }
            Err(e) => {
                let error_str = e.to_string();
                if error_str.contains("404") || error_str.contains("No such object") {
                    warn!("No existing state found in GCS, starting fresh");
                    Ok(None)
                } else {
                    Err(PublishError::Storage(format!("Failed to load state: {e}")))
                }
            }
        }
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
}
