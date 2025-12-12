//! HTTP fetcher for the sequencer's local storage API
//!
//! Fetches signed messages from a sequencer running with `--local-storage-path`.
//! The sequencer exposes a `/storage/*` API that this fetcher consumes.
//!
//! # Endpoints
//!
//! - `GET /storage/batches` - List all batches (for building batch index)
//! - `GET /storage/batches/{start}` - Fetch a batch by start sequence
//! - `GET /storage/messages/{sequence}` - Fetch a single message
//! - `GET /storage/latest` - Get the latest published sequence number

use crate::sync::fetcher::{BatchInfo, StorageFetcher};
use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use synddb_shared::types::message::{SignedBatch, SignedMessage};
use tracing::{debug, info, warn};

/// Response from the `/storage/latest` endpoint
#[derive(Debug, Deserialize)]
struct LatestSequenceResponse {
    sequence: Option<u64>,
}

/// Response from the `/storage/batches` list endpoint
#[derive(Debug, Deserialize)]
struct BatchInfoResponse {
    start_sequence: u64,
    end_sequence: u64,
}

/// HTTP fetcher for sequencer's local storage API
#[derive(Debug)]
pub struct HttpFetcher {
    client: Client,
    base_url: String,
}

impl HttpFetcher {
    /// Create a new HTTP fetcher
    ///
    /// # Arguments
    ///
    /// * `base_url` - Base URL of the sequencer (e.g., `http://localhost:8433`)
    pub fn new(base_url: impl Into<String>) -> Self {
        let base_url = base_url.into();
        let base_url = base_url.trim_end_matches('/').to_string();

        info!(base_url = %base_url, "HTTP fetcher initialized");

        Self {
            client: Client::new(),
            base_url,
        }
    }

    /// Get the URL for fetching a message
    fn message_url(&self, sequence: u64) -> String {
        format!("{}/storage/messages/{}", self.base_url, sequence)
    }

    /// Get the URL for fetching latest sequence
    fn latest_url(&self) -> String {
        format!("{}/storage/latest", self.base_url)
    }

    /// Get the URL for listing batches
    fn batches_url(&self) -> String {
        format!("{}/storage/batches", self.base_url)
    }

    /// Get the URL for fetching a specific batch (JSON format)
    fn batch_url(&self, start_sequence: u64) -> String {
        format!("{}/storage/batches/{}/json", self.base_url, start_sequence)
    }
}

#[async_trait]
impl StorageFetcher for HttpFetcher {
    fn name(&self) -> &str {
        "http"
    }

    fn supports_batches(&self) -> bool {
        true
    }

    async fn get(&self, sequence: u64) -> Result<Option<SignedMessage>> {
        let url = self.message_url(sequence);

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .with_context(|| format!("Failed to fetch message {sequence} from {url}"))?;

        match response.status() {
            status if status.is_success() => {
                let message: SignedMessage = response
                    .json()
                    .await
                    .with_context(|| format!("Failed to parse message {sequence}"))?;
                debug!(sequence, "Fetched message from HTTP");
                Ok(Some(message))
            }
            reqwest::StatusCode::NOT_FOUND => {
                debug!(sequence, "Message not found");
                Ok(None)
            }
            status => {
                let body = response.text().await.unwrap_or_default();
                Err(anyhow::anyhow!(
                    "HTTP error {status} fetching message {sequence}: {body}"
                ))
            }
        }
    }

    async fn get_latest_sequence(&self) -> Result<Option<u64>> {
        let url = self.latest_url();

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .with_context(|| format!("Failed to fetch latest sequence from {url}"))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!(
                "HTTP error {status} fetching latest sequence: {body}"
            ));
        }

        let latest: LatestSequenceResponse = response
            .json()
            .await
            .context("Failed to parse latest sequence response")?;

        if let Some(seq) = latest.sequence {
            debug!(sequence = seq, "Found latest sequence via HTTP");
        } else {
            warn!("No messages found via HTTP");
        }

        Ok(latest.sequence)
    }

    async fn list_batches(&self) -> Result<Vec<BatchInfo>> {
        let url = self.batches_url();

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .with_context(|| format!("Failed to list batches from {url}"))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!(
                "HTTP error {status} listing batches: {body}"
            ));
        }

        let batches: Vec<BatchInfoResponse> = response
            .json()
            .await
            .context("Failed to parse batches list response")?;

        let infos: Vec<BatchInfo> = batches
            .into_iter()
            .map(|b| {
                BatchInfo::new(
                    b.start_sequence,
                    b.end_sequence,
                    b.start_sequence.to_string(),
                )
            })
            .collect();

        debug!(count = infos.len(), "Listed batches via HTTP");
        Ok(infos)
    }

    async fn get_batch(&self, start_sequence: u64) -> Result<Option<SignedBatch>> {
        let url = self.batch_url(start_sequence);

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .with_context(|| format!("Failed to fetch batch {start_sequence} from {url}"))?;

        match response.status() {
            status if status.is_success() => {
                let batch: SignedBatch = response
                    .json()
                    .await
                    .with_context(|| format!("Failed to parse batch {start_sequence}"))?;
                debug!(
                    start = batch.start_sequence,
                    end = batch.end_sequence,
                    messages = batch.messages.len(),
                    "Fetched batch from HTTP"
                );
                Ok(Some(batch))
            }
            reqwest::StatusCode::NOT_FOUND => {
                debug!(start_sequence, "Batch not found");
                Ok(None)
            }
            status => {
                let body = response.text().await.unwrap_or_default();
                Err(anyhow::anyhow!(
                    "HTTP error {status} fetching batch {start_sequence}: {body}"
                ))
            }
        }
    }

    async fn get_batch_by_path(&self, path: &str) -> Result<Option<SignedBatch>> {
        // For HTTP, path is just the start_sequence as string
        let start_sequence: u64 = path
            .parse()
            .with_context(|| format!("Invalid batch path: {path}"))?;
        self.get_batch(start_sequence).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_url_construction() {
        let fetcher = HttpFetcher::new("http://localhost:8433");
        assert_eq!(
            fetcher.message_url(42),
            "http://localhost:8433/storage/messages/42"
        );
        assert_eq!(fetcher.latest_url(), "http://localhost:8433/storage/latest");
        assert_eq!(
            fetcher.batches_url(),
            "http://localhost:8433/storage/batches"
        );
        assert_eq!(
            fetcher.batch_url(1),
            "http://localhost:8433/storage/batches/1/json"
        );
    }

    #[test]
    fn test_url_trailing_slash() {
        let fetcher = HttpFetcher::new("http://localhost:8433/");
        assert_eq!(
            fetcher.message_url(1),
            "http://localhost:8433/storage/messages/1"
        );
        assert_eq!(
            fetcher.batches_url(),
            "http://localhost:8433/storage/batches"
        );
        assert_eq!(
            fetcher.batch_url(1),
            "http://localhost:8433/storage/batches/1/json"
        );
    }

    #[test]
    fn test_supports_batches() {
        let fetcher = HttpFetcher::new("http://localhost:8433");
        assert!(fetcher.supports_batches());
    }
}
