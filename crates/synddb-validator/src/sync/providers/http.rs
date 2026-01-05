//! HTTP fetcher for the sequencer's local storage API
//!
//! Fetches signed messages from a sequencer running with `--local-storage-path`.
//! The sequencer exposes a `/storage/*` API that this fetcher consumes.
//!
//! # Endpoints
//!
//! - `GET /storage/batches` - List all batches (for building batch index)
//! - `GET /storage/batches/{start}` - Fetch batch by start sequence (CBOR+zstd format)
//! - `GET /storage/messages/{sequence}` - Fetch a single message
//! - `GET /storage/latest` - Get the latest published sequence number

use crate::sync::fetcher::{BatchInfo, StorageFetcher};
use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use std::time::Duration;
use synddb_shared::types::{
    cbor::batch::CborBatch,
    message::{SignedBatch, SignedMessage},
};
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

/// Default timeout for HTTP requests (30 seconds)
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// Default maximum retries for transient failures
const DEFAULT_MAX_RETRIES: u32 = 3;

/// HTTP fetcher for sequencer's local storage API
#[derive(Debug)]
pub struct HttpFetcher {
    client: Client,
    base_url: String,
    max_retries: u32,
}

impl HttpFetcher {
    /// Create a new HTTP fetcher with default settings
    ///
    /// # Arguments
    ///
    /// * `base_url` - Base URL of the sequencer (e.g., `http://localhost:8433`)
    pub fn new(base_url: impl Into<String>) -> Self {
        Self::with_config(base_url, DEFAULT_TIMEOUT, DEFAULT_MAX_RETRIES)
    }

    /// Create a new HTTP fetcher with custom timeout and retry settings
    ///
    /// # Arguments
    ///
    /// * `base_url` - Base URL of the sequencer (e.g., `http://localhost:8433`)
    /// * `timeout` - HTTP request timeout
    /// * `max_retries` - Maximum retries for transient failures
    pub fn with_config(base_url: impl Into<String>, timeout: Duration, max_retries: u32) -> Self {
        let base_url = base_url.into();
        let base_url = base_url.trim_end_matches('/').to_string();

        let client = Client::builder()
            .timeout(timeout)
            .build()
            .expect("Failed to build HTTP client");

        info!(
            base_url = %base_url,
            timeout_secs = timeout.as_secs(),
            max_retries = max_retries,
            "HTTP fetcher initialized"
        );

        Self {
            client,
            base_url,
            max_retries,
        }
    }

    /// Check if an error is retryable (transient network issues)
    fn is_retryable_error(error: &reqwest::Error) -> bool {
        error.is_timeout() || error.is_connect() || error.is_request()
    }

    /// Execute a request with retry logic
    async fn execute_with_retry<T, F, Fut>(&self, operation: &str, f: F) -> Result<T>
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        let mut last_error = None;
        let mut delay = Duration::from_millis(100);

        for attempt in 0..=self.max_retries {
            match f().await {
                Ok(result) => return Ok(result),
                Err(e) => {
                    // Check if we should retry
                    let should_retry = attempt < self.max_retries
                        && e.downcast_ref::<reqwest::Error>()
                            .is_some_and(Self::is_retryable_error);

                    if should_retry {
                        warn!(
                            operation = operation,
                            attempt = attempt + 1,
                            max_retries = self.max_retries,
                            error = %e,
                            delay_ms = delay.as_millis(),
                            "Retrying after transient error"
                        );
                        tokio::time::sleep(delay).await;
                        delay = std::cmp::min(delay * 2, Duration::from_secs(10));
                    }

                    last_error = Some(e);
                }
            }
        }

        Err(last_error.unwrap())
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

    /// Get the URL for fetching a specific batch (CBOR+zstd format)
    fn batch_url(&self, start_sequence: u64) -> String {
        format!("{}/storage/batches/{}", self.base_url, start_sequence)
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
        let client = self.client.clone();

        self.execute_with_retry(&format!("get_message_{sequence}"), || {
            let url = url.clone();
            let client = client.clone();
            async move {
                let response =
                    client.get(&url).send().await.with_context(|| {
                        format!("Failed to fetch message {sequence} from {url}")
                    })?;

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
        })
        .await
    }

    async fn get_latest_sequence(&self) -> Result<Option<u64>> {
        let url = self.latest_url();
        let client = self.client.clone();

        self.execute_with_retry("get_latest_sequence", || {
            let url = url.clone();
            let client = client.clone();
            async move {
                let response = client
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
                    debug!("No messages found via HTTP");
                }

                Ok(latest.sequence)
            }
        })
        .await
    }

    async fn list_batches(&self) -> Result<Vec<BatchInfo>> {
        let url = self.batches_url();
        let client = self.client.clone();

        self.execute_with_retry("list_batches", || {
            let url = url.clone();
            let client = client.clone();
            async move {
                let response = client
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
        })
        .await
    }

    async fn get_batch(&self, start_sequence: u64) -> Result<Option<SignedBatch>> {
        let url = self.batch_url(start_sequence);
        let client = self.client.clone();

        self.execute_with_retry(&format!("get_batch_{start_sequence}"), || {
            let url = url.clone();
            let client = client.clone();
            async move {
                let response = client.get(&url).send().await.with_context(|| {
                    format!("Failed to fetch batch {start_sequence} from {url}")
                })?;

                match response.status() {
                    status if status.is_success() => {
                        let data = response.bytes().await.with_context(|| {
                            format!("Failed to read batch {start_sequence} body")
                        })?;

                        // Parse CBOR+zstd format
                        let cbor_batch = CborBatch::from_cbor_zstd(&data).with_context(|| {
                            format!("Failed to parse CBOR batch {start_sequence}")
                        })?;

                        // Convert to SignedBatch
                        let batch = cbor_batch.to_signed_batch().with_context(|| {
                            format!("Failed to convert CBOR batch {start_sequence}")
                        })?;

                        debug!(
                            start = batch.start_sequence,
                            end = batch.end_sequence,
                            messages = batch.messages.len(),
                            "Fetched CBOR batch from HTTP"
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
        })
        .await
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
            "http://localhost:8433/storage/batches/1"
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
            "http://localhost:8433/storage/batches/1"
        );
    }

    #[test]
    fn test_supports_batches() {
        let fetcher = HttpFetcher::new("http://localhost:8433");
        assert!(fetcher.supports_batches());
    }
}
