use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use base64::Engine;
use serde::{Deserialize, Serialize};
use synddb_shared::types::message::{SignedBatch, SignedMessage};
use url::Url;

/// Sequencer status response
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct SequencerStatus {
    pub current_sequence: u64,
    pub signer_address: String,
}

/// Storage latest response
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct StorageLatest {
    pub sequence: Option<u64>,
}

/// Batch info response from /storage/batches
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct BatchInfo {
    pub start_sequence: u64,
    pub end_sequence: u64,
}

/// Sequence response from sequencer
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct SequenceResponse {
    pub sequence: u64,
    pub timestamp: u64,
    pub message_hash: String,
    pub signature: String,
    pub signer: String,
}

/// HTTP client for sequencer API
#[derive(Debug)]
pub struct SequencerClient {
    base_url: Url,
    client: reqwest::Client,
}

impl SequencerClient {
    pub(crate) fn new(base_url: &str) -> Self {
        let base_url = Url::parse(base_url).expect("Invalid sequencer URL");
        Self {
            base_url,
            client: reqwest::Client::new(),
        }
    }

    /// Get sequencer status
    pub(crate) async fn status(&self) -> Result<SequencerStatus> {
        let url = self.base_url.join("/status")?;
        self.client
            .get(url)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
            .context("Failed to parse sequencer status")
    }

    /// Fetch a message from the storage layer
    pub(crate) async fn fetch_storage_message(&self, sequence: u64) -> Result<SignedMessage> {
        let url = self
            .base_url
            .join(&format!("/storage/messages/{}", sequence))?;
        self.client
            .get(url)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
            .context("Failed to fetch storage message")
    }

    /// Get the latest storage sequence
    pub(crate) async fn storage_latest(&self) -> Result<StorageLatest> {
        let url = self.base_url.join("/storage/latest")?;
        self.client
            .get(url)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
            .context("Failed to get storage latest")
    }

    /// List all batches from the storage layer
    pub(crate) async fn list_batches(&self) -> Result<Vec<BatchInfo>> {
        let url = self.base_url.join("/storage/batches")?;
        self.client
            .get(url)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
            .context("Failed to list storage batches")
    }

    /// Fetch a batch by start sequence
    pub(crate) async fn fetch_batch(&self, start_sequence: u64) -> Result<SignedBatch> {
        let url = self
            .base_url
            .join(&format!("/storage/batches/{}", start_sequence))?;
        self.client
            .get(url)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
            .context("Failed to fetch storage batch")
    }

    /// Try to fetch a batch, returning None if not found (404)
    pub(crate) async fn try_fetch_batch(&self, start_sequence: u64) -> Result<Option<SignedBatch>> {
        let url = self
            .base_url
            .join(&format!("/storage/batches/{}", start_sequence))?;
        let response = self.client.get(url).send().await?;

        match response.status() {
            status if status.is_success() => {
                let batch = response.json().await?;
                Ok(Some(batch))
            }
            reqwest::StatusCode::NOT_FOUND => Ok(None),
            status => {
                let body = response.text().await.unwrap_or_default();
                bail!("HTTP error {}: {}", status, body)
            }
        }
    }

    /// Try to fetch a message, returning None if not found (404)
    pub(crate) async fn try_fetch_storage_message(
        &self,
        sequence: u64,
    ) -> Result<Option<SignedMessage>> {
        let url = self
            .base_url
            .join(&format!("/storage/messages/{}", sequence))?;
        let response = self.client.get(url).send().await?;

        match response.status() {
            status if status.is_success() => {
                let message = response.json().await?;
                Ok(Some(message))
            }
            reqwest::StatusCode::NOT_FOUND => Ok(None),
            status => {
                let body = response.text().await.unwrap_or_default();
                bail!("HTTP error {}: {}", status, body)
            }
        }
    }

    /// List all batches from the DA layer
    pub(crate) async fn list_batches(&self) -> Result<Vec<BatchInfo>> {
        let url = self.base_url.join("/da/batches")?;
        self.client
            .get(url)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
            .context("Failed to list DA batches")
    }

    /// Fetch a batch by start sequence
    pub(crate) async fn fetch_batch(&self, start_sequence: u64) -> Result<SignedBatch> {
        let url = self
            .base_url
            .join(&format!("/da/batches/{}", start_sequence))?;
        self.client
            .get(url)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
            .context("Failed to fetch DA batch")
    }

    /// Try to fetch a batch, returning None if not found (404)
    pub(crate) async fn try_fetch_batch(&self, start_sequence: u64) -> Result<Option<SignedBatch>> {
        let url = self
            .base_url
            .join(&format!("/da/batches/{}", start_sequence))?;
        let response = self.client.get(url).send().await?;

        match response.status() {
            status if status.is_success() => {
                let batch = response.json().await?;
                Ok(Some(batch))
            }
            reqwest::StatusCode::NOT_FOUND => Ok(None),
            status => {
                let body = response.text().await.unwrap_or_default();
                bail!("HTTP error {}: {}", status, body)
            }
        }
    }

    /// Try to fetch a message, returning None if not found (404)
    pub(crate) async fn try_fetch_da_message(
        &self,
        sequence: u64,
    ) -> Result<Option<SignedMessage>> {
        let url = self.base_url.join(&format!("/da/messages/{}", sequence))?;
        let response = self.client.get(url).send().await?;

        match response.status() {
            status if status.is_success() => {
                let message = response.json().await?;
                Ok(Some(message))
            }
            reqwest::StatusCode::NOT_FOUND => Ok(None),
            status => {
                let body = response.text().await.unwrap_or_default();
                bail!("HTTP error {}: {}", status, body)
            }
        }
    }

    /// Wait for the sequencer to be healthy
    pub(crate) async fn wait_healthy(&self, timeout: Duration) -> Result<()> {
        let start = Instant::now();
        let url = self.base_url.join("/health")?;

        while start.elapsed() < timeout {
            if self.client.get(url.clone()).send().await.is_ok() {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }

        bail!("Sequencer did not become healthy within {:?}", timeout)
    }

    /// Send a snapshot to the sequencer
    pub(crate) async fn send_snapshot(
        &self,
        message_id: &str,
        data: &[u8],
        client_sequence: u64,
    ) -> Result<SequenceResponse> {
        let url = self.base_url.join("/snapshots")?;

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let body = serde_json::json!({
            "message_id": message_id,
            "snapshot": {
                "data": base64::engine::general_purpose::STANDARD.encode(data),
                "timestamp": timestamp,
                "sequence": client_sequence
            }
        });

        self.client
            .post(url)
            .json(&body)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
            .context("Failed to send snapshot")
    }
}
