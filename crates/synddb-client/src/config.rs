//! Configuration for `SyndDB` client

use crate::attestation::TokenType;
use clap::Parser;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use synddb_chain_monitor::config::ChainMonitorConfig;

#[derive(Debug, Clone, Serialize, Deserialize, Parser)]
#[command(author, version, about, long_about = None)]
pub struct Config {
    /// URL of the sequencer TEE
    #[arg(long, env = "SEQUENCER_URL", default_value = "http://localhost:8433")]
    pub sequencer_url: String,

    /// Maximum number of changesets to buffer before publishing
    #[arg(long, env = "BUFFER_SIZE", default_value = "100")]
    #[serde(default = "inline_default_buffer_size")]
    pub buffer_size: usize,

    /// Maximum time to wait before publishing buffer (e.g., "1s")
    #[arg(long, env = "PUBLISH_INTERVAL", default_value = "1s", value_parser = parse_duration)]
    #[serde(with = "humantime_serde")]
    #[serde(default = "inline_default_publish_interval")]
    pub publish_interval: Duration,

    /// Maximum size of a changeset batch in bytes
    #[arg(long, env = "MAX_BATCH_SIZE", default_value = "1048576")]
    #[serde(default = "inline_default_max_batch_size")]
    pub max_batch_size: usize,

    /// Number of retries for failed sends
    #[arg(long, env = "MAX_RETRIES", default_value = "3")]
    #[serde(default = "inline_default_max_retries")]
    pub max_retries: usize,

    /// Timeout for HTTP requests (e.g., "10s")
    #[arg(long, env = "REQUEST_TIMEOUT", default_value = "10s", value_parser = parse_duration)]
    #[serde(with = "humantime_serde")]
    #[serde(default = "inline_default_request_timeout")]
    pub request_timeout: Duration,

    /// Number of changesets between automatic snapshots (0 to disable)
    #[arg(long, env = "SNAPSHOT_INTERVAL", default_value = "100")]
    #[serde(default = "inline_default_snapshot_interval")]
    pub snapshot_interval: u64,

    /// Enable recovery storage for failed batches
    #[arg(long, env = "ENABLE_RECOVERY", default_value = "true")]
    #[serde(default = "inline_default_enable_recovery")]
    pub enable_recovery: bool,

    /// Enable TEE attestation tokens for GCP Confidential Space
    #[arg(long, env = "ENABLE_ATTESTATION", default_value = "false")]
    #[serde(default = "inline_default_enable_attestation")]
    pub enable_attestation: bool,

    /// Type of attestation token (oidc, pki, or aws-principal-tags)
    #[arg(long, env = "ATTESTATION_TOKEN_TYPE", default_value = "oidc", value_parser = parse_token_type)]
    #[serde(default = "inline_default_attestation_token_type")]
    pub attestation_token_type: TokenType,

    /// Chain monitor configuration (optional, only enabled with "chain-monitor" feature)
    #[arg(skip)]
    #[serde(default)]
    pub chain_monitor: Option<ChainMonitorConfig>,
}

// Custom parsers for clap
fn parse_duration(s: &str) -> Result<Duration, String> {
    humantime::parse_duration(s).map_err(|e| format!("Invalid duration: {}", e))
}

fn parse_token_type(s: &str) -> Result<TokenType, String> {
    match s.to_lowercase().as_str() {
        "oidc" => Ok(TokenType::Oidc),
        "pki" => Ok(TokenType::Pki),
        "aws-principal-tags" | "aws_principaltags" | "awsprincipaltags" => {
            Ok(TokenType::AwsPrincipaltags)
        }
        _ => Err(format!(
            "Invalid token type '{}'. Expected: oidc, pki, or aws-principal-tags",
            s
        )),
    }
}

// Inline defaults for serde deserialization
const fn inline_default_buffer_size() -> usize {
    100
}

const fn inline_default_publish_interval() -> Duration {
    Duration::from_secs(1)
}

const fn inline_default_max_batch_size() -> usize {
    1024 * 1024 // 1MB
}

const fn inline_default_max_retries() -> usize {
    3
}

const fn inline_default_request_timeout() -> Duration {
    Duration::from_secs(10)
}

const fn inline_default_snapshot_interval() -> u64 {
    100
}

const fn inline_default_enable_recovery() -> bool {
    true
}

const fn inline_default_enable_attestation() -> bool {
    false
}

const fn inline_default_attestation_token_type() -> TokenType {
    TokenType::Oidc
}

impl Default for Config {
    fn default() -> Self {
        Self {
            sequencer_url: "http://localhost:8433".to_string(),
            buffer_size: inline_default_buffer_size(),
            publish_interval: inline_default_publish_interval(),
            max_batch_size: inline_default_max_batch_size(),
            max_retries: inline_default_max_retries(),
            request_timeout: inline_default_request_timeout(),
            snapshot_interval: inline_default_snapshot_interval(),
            enable_recovery: inline_default_enable_recovery(),
            enable_attestation: inline_default_enable_attestation(),
            attestation_token_type: inline_default_attestation_token_type(),
            chain_monitor: None,
        }
    }
}
