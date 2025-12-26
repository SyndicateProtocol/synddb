use anyhow::{Context, Result};
use async_trait::async_trait;
use google_cloud_storage::client::{Client, ClientConfig};
use google_cloud_storage::http::objects::upload::{Media, UploadObjectRequest, UploadType};

use crate::storage::{StoragePublisher, StorageRecord};

pub struct GcsPublisher {
    client: Client,
    bucket: String,
}

impl GcsPublisher {
    pub async fn new(bucket: String) -> Result<Self> {
        let config = ClientConfig::default()
            .with_auth()
            .await
            .context("Failed to configure GCS client")?;

        let client = Client::new(config);

        Ok(Self { client, bucket })
    }

    fn object_path(message_id: &[u8; 32]) -> String {
        format!("messages/{}.json", hex::encode(message_id))
    }
}

#[async_trait]
impl StoragePublisher for GcsPublisher {
    async fn publish(&self, record: &StorageRecord) -> Result<String> {
        let path = Self::object_path(&record.message.id);
        let json = serde_json::to_vec_pretty(record).context("Failed to serialize record")?;

        let upload_type = UploadType::Simple(Media::new(path.clone()));

        self.client
            .upload_object(
                &UploadObjectRequest {
                    bucket: self.bucket.clone(),
                    ..Default::default()
                },
                json,
                &upload_type,
            )
            .await
            .context("Failed to upload to GCS")?;

        Ok(format!("gcs://{}/{}", self.bucket, path))
    }

    fn uri_prefix(&self) -> &str {
        "gcs://"
    }
}
