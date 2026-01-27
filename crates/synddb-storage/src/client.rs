//! Storage client implementation

use crate::{config::StorageConfig, error::StorageError};
use bytes::Bytes;
use google_cloud_storage::client::{Storage, StorageControl};
use tracing::{debug, info};

/// A client for object storage operations
///
/// Currently supports Google Cloud Storage with emulator support for testing.
pub struct StorageClient {
    storage: Storage,
    control: StorageControl,
    config: StorageConfig,
    /// HTTP client for emulator REST API (`StorageControl` uses gRPC which fake-gcs-server doesn't support)
    emulator_client: Option<reqwest::Client>,
}

impl std::fmt::Debug for StorageClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StorageClient")
            .field("bucket", &self.config.bucket())
            .field("prefix", &self.config.prefix())
            .field("emulator", &self.config.is_emulator())
            .finish()
    }
}

/// Information about a listed object
#[derive(Debug, Clone)]
pub struct ObjectInfo {
    /// Full path/name of the object
    pub name: String,
}

impl StorageClient {
    /// Create a new storage client
    pub async fn new(config: StorageConfig) -> Result<Self, StorageError> {
        match &config {
            StorageConfig::Gcs(gcs_config) => Self::new_gcs(config.clone(), gcs_config).await,
        }
    }

    async fn new_gcs(
        config: StorageConfig,
        gcs_config: &synddb_shared::gcs::GcsConfig,
    ) -> Result<Self, StorageError> {
        let (storage, control, emulator_client) =
            if let Some(ref emulator_host) = gcs_config.emulator_host {
                info!(emulator_host = %emulator_host, "Using GCS emulator");

                use google_cloud_auth::credentials::anonymous;
                let anonymous_creds = anonymous::Builder::default().build();

                let storage = Storage::builder()
                    .with_endpoint(emulator_host)
                    .with_credentials(anonymous_creds.clone())
                    .build()
                    .await
                    .map_err(|e| {
                        StorageError::Config(format!("Failed to create Storage client: {e}"))
                    })?;

                let control = StorageControl::builder()
                    .with_endpoint(emulator_host)
                    .with_credentials(anonymous_creds)
                    .build()
                    .await
                    .map_err(|e| {
                        StorageError::Config(format!("Failed to create StorageControl client: {e}"))
                    })?;

                let http_client = reqwest::Client::new();

                (storage, control, Some(http_client))
            } else {
                let storage = Storage::builder().build().await.map_err(|e| {
                    StorageError::Config(format!("Failed to create Storage client: {e}"))
                })?;

                let control = StorageControl::builder().build().await.map_err(|e| {
                    StorageError::Config(format!("Failed to create StorageControl client: {e}"))
                })?;

                (storage, control, None)
            };

        info!(
            bucket = %gcs_config.bucket,
            prefix = %gcs_config.prefix,
            "Storage client initialized"
        );

        Ok(Self {
            storage,
            control,
            config,
            emulator_client,
        })
    }

    /// Get the bucket path in the format required by the Google Cloud Storage SDK
    fn bucket_path(&self) -> String {
        format!("projects/_/buckets/{}", self.config.bucket())
    }

    /// Write data to storage
    pub async fn write(&self, path: &str, data: impl Into<Bytes>) -> Result<(), StorageError> {
        self.storage
            .write_object(self.bucket_path(), path, data.into())
            .send_buffered()
            .await
            .map_err(|e| StorageError::Write(format!("Failed to write to {path}: {e}")))?;

        debug!(path, "Wrote object to storage");
        Ok(())
    }

    /// Read data from storage
    ///
    /// Returns `None` if the object doesn't exist.
    pub async fn read(&self, path: &str) -> Result<Option<Vec<u8>>, StorageError> {
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
                return Err(StorageError::Read(format!("Failed to read {path}: {e}")));
            }
        };

        let mut data = Vec::new();
        while let Some(chunk) = reader.next().await {
            let chunk =
                chunk.map_err(|e| StorageError::Read(format!("Failed to read chunk: {e}")))?;
            data.extend_from_slice(&chunk);
        }

        debug!(path, bytes = data.len(), "Read object from storage");
        Ok(Some(data))
    }

    /// List objects with the given prefix
    ///
    /// Returns object names (full paths).
    pub async fn list(&self, prefix: &str) -> Result<Vec<ObjectInfo>, StorageError> {
        // Use REST API for emulator mode (StorageControl uses gRPC which fake-gcs-server doesn't support)
        if let Some(ref client) = self.emulator_client {
            return self.list_emulator(client, prefix).await;
        }

        let mut objects = Vec::new();
        let mut page_token: Option<String> = None;

        loop {
            let mut request = self
                .control
                .list_objects()
                .set_parent(self.bucket_path())
                .set_prefix(prefix);

            if let Some(ref token) = page_token {
                request = request.set_page_token(token);
            }

            let response = request
                .send()
                .await
                .map_err(|e| StorageError::List(format!("Failed to list objects: {e}")))?;

            for obj in response.objects {
                objects.push(ObjectInfo { name: obj.name });
            }

            if response.next_page_token.is_empty() {
                break;
            }
            debug!(token = %response.next_page_token, "Fetching next page");
            page_token = Some(response.next_page_token);
        }

        debug!(prefix, count = objects.len(), "Listed objects");
        Ok(objects)
    }

    /// List objects using the GCS REST API (for emulator mode)
    async fn list_emulator(
        &self,
        client: &reqwest::Client,
        prefix: &str,
    ) -> Result<Vec<ObjectInfo>, StorageError> {
        let emulator_host = self
            .config
            .emulator_host()
            .expect("emulator_client exists only when emulator_host is set");

        let mut objects = Vec::new();
        let mut page_token: Option<String> = None;

        loop {
            use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
            let encoded_prefix = utf8_percent_encode(prefix, NON_ALPHANUMERIC).to_string();
            let mut url = format!(
                "{}/storage/v1/b/{}/o?prefix={}",
                emulator_host,
                self.config.bucket(),
                encoded_prefix
            );
            if let Some(ref token) = page_token {
                let encoded_token = utf8_percent_encode(token, NON_ALPHANUMERIC).to_string();
                url.push_str(&format!("&pageToken={}", encoded_token));
            }

            let response = client.get(&url).send().await?;

            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                return Err(StorageError::List(format!(
                    "Emulator list failed: {} - {}",
                    status, body
                )));
            }

            let body: serde_json::Value = response
                .json()
                .await
                .map_err(|e| StorageError::List(format!("Failed to parse response: {e}")))?;

            if let Some(items) = body.get("items").and_then(|v| v.as_array()) {
                for item in items {
                    if let Some(name) = item.get("name").and_then(|v| v.as_str()) {
                        objects.push(ObjectInfo {
                            name: name.to_string(),
                        });
                    }
                }
            }

            match body.get("nextPageToken").and_then(|v| v.as_str()) {
                Some(token) if !token.is_empty() => {
                    debug!(token = %token, "Fetching next page (emulator)");
                    page_token = Some(token.to_string());
                }
                _ => break,
            }
        }

        debug!(prefix, count = objects.len(), "Listed objects (emulator)");
        Ok(objects)
    }

    /// List objects with prefix and return only the first match
    pub async fn list_one(&self, prefix: &str) -> Result<Option<ObjectInfo>, StorageError> {
        if let Some(ref client) = self.emulator_client {
            return self.list_one_emulator(client, prefix).await;
        }

        let response = self
            .control
            .list_objects()
            .set_parent(self.bucket_path())
            .set_prefix(prefix)
            .send()
            .await
            .map_err(|e| StorageError::List(format!("Failed to list objects: {e}")))?;

        Ok(response
            .objects
            .into_iter()
            .next()
            .map(|obj| ObjectInfo { name: obj.name }))
    }

    /// List one object using REST API (for emulator mode)
    async fn list_one_emulator(
        &self,
        client: &reqwest::Client,
        prefix: &str,
    ) -> Result<Option<ObjectInfo>, StorageError> {
        let emulator_host = self
            .config
            .emulator_host()
            .expect("emulator_client exists only when emulator_host is set");

        use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
        let encoded_prefix = utf8_percent_encode(prefix, NON_ALPHANUMERIC).to_string();
        let url = format!(
            "{}/storage/v1/b/{}/o?prefix={}&maxResults=1",
            emulator_host,
            self.config.bucket(),
            encoded_prefix
        );

        let response = client.get(&url).send().await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(StorageError::List(format!(
                "Emulator list failed: {} - {}",
                status, body
            )));
        }

        let body: serde_json::Value = response
            .json()
            .await
            .map_err(|e| StorageError::List(format!("Failed to parse response: {e}")))?;

        Ok(body
            .get("items")
            .and_then(|v| v.as_array())
            .and_then(|items| items.first())
            .and_then(|item| item.get("name"))
            .and_then(|v| v.as_str())
            .map(|name| ObjectInfo {
                name: name.to_string(),
            }))
    }

    /// Get the configured prefix
    pub fn prefix(&self) -> &str {
        self.config.prefix()
    }

    /// Get the configured bucket
    pub fn bucket(&self) -> &str {
        self.config.bucket()
    }
}
