//! Configuration for SyndDB client

use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// URL of the sequencer TEE
    pub sequencer_url: String,

    /// Maximum number of changesets to buffer before publishing
    #[serde(default = "default_buffer_size")]
    pub buffer_size: usize,

    /// Maximum time to wait before publishing buffer
    #[serde(with = "humantime_serde")]
    #[serde(default = "default_publish_interval")]
    pub publish_interval: Duration,

    /// Maximum size of a changeset batch in bytes
    #[serde(default = "default_max_batch_size")]
    pub max_batch_size: usize,

    /// Number of retries for failed sends
    #[serde(default = "default_max_retries")]
    pub max_retries: usize,

    /// Timeout for HTTP requests
    #[serde(with = "humantime_serde")]
    #[serde(default = "default_request_timeout")]
    pub request_timeout: Duration,

    /// Number of changesets between automatic snapshots
    /// Set to 0 to disable automatic snapshots
    #[serde(default = "default_snapshot_interval")]
    pub snapshot_interval: u64,
}

fn default_buffer_size() -> usize {
    100
}

fn default_publish_interval() -> Duration {
    Duration::from_secs(1)
}

fn default_max_batch_size() -> usize {
    1024 * 1024 // 1MB
}

fn default_max_retries() -> usize {
    3
}

fn default_request_timeout() -> Duration {
    Duration::from_secs(10)
}

fn default_snapshot_interval() -> u64 {
    100 // Snapshot every 100 changesets
}

impl Default for Config {
    fn default() -> Self {
        Self {
            sequencer_url: "http://localhost:8433".to_string(),
            buffer_size: default_buffer_size(),
            publish_interval: default_publish_interval(),
            max_batch_size: default_max_batch_size(),
            max_retries: default_max_retries(),
            request_timeout: default_request_timeout(),
            snapshot_interval: default_snapshot_interval(),
        }
    }
}
