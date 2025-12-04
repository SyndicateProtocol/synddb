use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use url::Url;

/// Validator status response
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct ValidatorStatus {
    pub running: bool,
    pub last_sequence: Option<u64>,
    pub last_sync_time: u64,
    pub next_sequence: u64,
}

/// HTTP client for validator API
pub(crate) struct ValidatorClient {
    base_url: Url,
    client: reqwest::Client,
}

impl ValidatorClient {
    pub(crate) fn new(base_url: &str) -> Self {
        let base_url = Url::parse(base_url).expect("Invalid validator URL");
        Self {
            base_url,
            client: reqwest::Client::new(),
        }
    }

    /// Get validator status
    pub(crate) async fn status(&self) -> Result<ValidatorStatus> {
        let url = self.base_url.join("/status")?;
        self.client
            .get(url)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
            .context("Failed to parse validator status")
    }

    /// Wait for the validator to be healthy
    pub(crate) async fn wait_healthy(&self, timeout: Duration) -> Result<()> {
        let start = Instant::now();
        let url = self.base_url.join("/health")?;

        while start.elapsed() < timeout {
            if self.client.get(url.clone()).send().await.is_ok() {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }

        bail!("Validator did not become healthy within {:?}", timeout)
    }
}
