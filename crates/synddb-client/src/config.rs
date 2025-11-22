//! Configuration for `SyndDB` client

use crate::attestation::TokenType;
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

    /// Enable recovery storage for failed batches
    /// When enabled, failed changesets and snapshots are saved to a local `SQLite` database for retry
    #[serde(default = "default_enable_recovery")]
    pub enable_recovery: bool,

    /// Enable TEE attestation tokens for GCP Confidential Space
    /// When enabled, each changeset batch and snapshot will include an attestation token
    #[serde(default = "default_enable_attestation")]
    pub enable_attestation: bool,

    /// Type of attestation token to request (OIDC, PKI, or `AWS_PRINCIPALTAGS`)
    /// Only used if `enable_attestation` is true
    #[serde(default = "default_attestation_token_type")]
    pub attestation_token_type: TokenType,

    /// Chain monitor configuration (optional, only enabled with "chain-monitor" feature)
    #[cfg(feature = "chain-monitor")]
    #[serde(default)]
    pub chain_monitor: Option<ChainMonitorConfig>,
}

/// Configuration for blockchain chain monitoring
#[cfg(feature = "chain-monitor")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainMonitorConfig {
    /// WebSocket RPC URLs for blockchain connection (multiple for redundancy)
    pub ws_urls: Vec<String>,

    /// Contract address to monitor for events
    pub contract_address: String,

    /// Block number to start monitoring from
    pub start_block: u64,

    /// Optional specific event signature to filter (leave None for all events)
    pub event_signature: Option<String>,

    /// Path to store event processing state
    #[serde(default = "default_event_store_path")]
    pub event_store_path: String,

    /// Database table name to insert deposit events into
    #[serde(default = "default_deposit_table")]
    pub deposit_table: String,
}

#[cfg(feature = "chain-monitor")]
fn default_event_store_path() -> String {
    "./chain_events.db".to_string()
}

#[cfg(feature = "chain-monitor")]
fn default_deposit_table() -> String {
    "deposits".to_string()
}

const fn default_buffer_size() -> usize {
    100
}

const fn default_publish_interval() -> Duration {
    Duration::from_secs(1)
}

const fn default_max_batch_size() -> usize {
    1024 * 1024 // 1MB
}

const fn default_max_retries() -> usize {
    3
}

const fn default_request_timeout() -> Duration {
    Duration::from_secs(10)
}

const fn default_snapshot_interval() -> u64 {
    100
}

const fn default_enable_recovery() -> bool {
    true
}

const fn default_enable_attestation() -> bool {
    false
}

const fn default_attestation_token_type() -> TokenType {
    TokenType::Oidc
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
            enable_recovery: default_enable_recovery(),
            enable_attestation: default_enable_attestation(),
            attestation_token_type: default_attestation_token_type(),
            #[cfg(feature = "chain-monitor")]
            chain_monitor: None,
        }
    }
}
