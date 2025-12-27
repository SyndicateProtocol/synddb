//! Configuration for `SyndDB` client

use crate::attestation::TokenType;
use clap::Parser;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use synddb_chain_monitor::config::ChainMonitorConfig;
use synddb_shared::parse::parse_url;
use url::Url;

#[derive(Debug, Clone, Serialize, Deserialize, Parser)]
#[command(author, version, about, long_about = None)]
pub struct Config {
    /// URL of the sequencer TEE (e.g., `http://localhost:8433`)
    #[arg(long, env = "SEQUENCER_URL", default_value = "http://localhost:8433", value_parser = parse_url)]
    pub sequencer_url: Url,

    /// Maximum number of changesets to buffer before sending
    #[arg(long, env = "BUFFER_SIZE", default_value = "100")]
    pub buffer_size: usize,

    /// Maximum time to buffer changesets before sending (e.g., "1s")
    #[arg(long, env = "FLUSH_INTERVAL", default_value = "1s", value_parser = humantime::parse_duration)]
    #[serde(with = "humantime_serde")]
    pub flush_interval: Duration,

    /// Maximum size of a changeset batch in bytes
    #[arg(long, env = "MAX_BATCH_SIZE", default_value = "1048576")]
    pub max_batch_size: usize,

    /// Number of retries for failed sends
    #[arg(long, env = "MAX_RETRIES", default_value = "3")]
    pub max_retries: usize,

    /// Timeout for HTTP requests (e.g., "10s")
    #[arg(long, env = "SNAPSHOT_REQUEST_TIMEOUT", default_value = "10s", value_parser = humantime::parse_duration)]
    #[serde(with = "humantime_serde")]
    pub snapshot_request_timeout: Duration,

    /// Number of changesets between automatic snapshots (must be > 0)
    #[arg(long, env = "SNAPSHOT_INTERVAL", default_value = "100")]
    pub snapshot_interval: u64,

    /// Enable recovery storage for failed batches
    #[arg(long, env = "ENABLE_RECOVERY")]
    pub enable_recovery: bool,

    /// Disable TEE attestation tokens (enabled by default for production)
    #[arg(long, env = "DISABLE_ATTESTATION")]
    pub disable_attestation: bool,

    /// Type of attestation token (oidc, pki, or aws-principal-tags)
    #[arg(long, env = "ATTESTATION_TOKEN_TYPE", default_value = "oidc", value_parser = parse_token_type)]
    pub attestation_token_type: TokenType,

    /// Chain monitor configuration (optional, only enabled with "chain-monitor" feature)
    #[command(flatten)]
    #[serde(default)]
    pub chain_monitor: Option<ChainMonitorConfig>,
}

impl Default for Config {
    fn default() -> Self {
        // Parse empty args to get default values from clap's `default_value` attributes
        Self::parse_from::<[&str; 0], &str>([])
    }
}

// Custom parsers for clap
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
