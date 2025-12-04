use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use synddb_shared::types::message::SignedMessage;
use url::Url;

/// Sequencer status response
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct SequencerStatus {
    pub current_sequence: u64,
    pub signer_address: String,
}

/// DA latest response
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct DaLatest {
    pub sequence: Option<u64>,
}

/// HTTP client for sequencer API
pub(crate) struct SequencerClient {
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

    /// Fetch a message from the DA layer
    pub(crate) async fn fetch_da_message(&self, sequence: u64) -> Result<SignedMessage> {
        let url = self.base_url.join(&format!("/da/messages/{}", sequence))?;
        self.client
            .get(url)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
            .context("Failed to fetch DA message")
    }

    /// Get the latest DA sequence
    pub(crate) async fn da_latest(&self) -> Result<DaLatest> {
        let url = self.base_url.join("/da/latest")?;
        self.client
            .get(url)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
            .context("Failed to get DA latest")
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
}
