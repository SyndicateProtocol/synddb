//! HTTP fetcher for the sequencer's local DA API
//!
//! Fetches signed messages from a sequencer running with `--local-storage-path`.
//! The sequencer exposes a `/da/*` API that this fetcher consumes.
//!
//! # Endpoints
//!
//! - `GET /da/messages/{sequence}` - Fetch a single message
//! - `GET /da/batches/{start}` - Fetch a batch by start sequence
//! - `GET /da/latest` - Get the latest published sequence number

use crate::sync::fetcher::DAFetcher;
use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use synddb_shared::types::message::SignedMessage;
use tracing::{debug, info, warn};

/// Response from the `/da/latest` endpoint
#[derive(Debug, Deserialize)]
struct LatestSequenceResponse {
    sequence: Option<u64>,
}

/// HTTP fetcher for sequencer's local DA API
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
        format!("{}/da/messages/{}", self.base_url, sequence)
    }

    /// Get the URL for fetching latest sequence
    fn latest_url(&self) -> String {
        format!("{}/da/latest", self.base_url)
    }
}

#[async_trait]
impl DAFetcher for HttpFetcher {
    fn name(&self) -> &str {
        "http"
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_url_construction() {
        let fetcher = HttpFetcher::new("http://localhost:8433");
        assert_eq!(
            fetcher.message_url(42),
            "http://localhost:8433/da/messages/42"
        );
        assert_eq!(fetcher.latest_url(), "http://localhost:8433/da/latest");
    }

    #[test]
    fn test_url_trailing_slash() {
        let fetcher = HttpFetcher::new("http://localhost:8433/");
        assert_eq!(
            fetcher.message_url(1),
            "http://localhost:8433/da/messages/1"
        );
    }
}
