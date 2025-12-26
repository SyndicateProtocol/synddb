use std::time::Duration;

use alloy::primitives::Address;
use clap::Parser;
use serde::{Deserialize, Serialize};

fn parse_duration_seconds(s: &str) -> Result<Duration, std::num::ParseIntError> {
    let secs: u64 = s.parse()?;
    Ok(Duration::from_secs(secs))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum ValidatorMode {
    #[default]
    Primary,
    Witness,
}

#[derive(Debug, Clone, Serialize, Deserialize, Parser)]
pub struct ValidatorConfig {
    #[arg(long, env = "VALIDATOR_MODE", default_value = "primary")]
    pub mode: ValidatorMode,

    #[arg(long, env = "BRIDGE_ADDRESS")]
    pub bridge_address: Address,

    #[arg(long, env = "BRIDGE_CHAIN_ID")]
    pub bridge_chain_id: u64,

    #[arg(long, env = "RPC_URL")]
    pub rpc_url: String,

    #[arg(long, env = "WS_URL")]
    pub ws_url: Option<String>,

    #[arg(long, env = "PRIVATE_KEY")]
    pub private_key: Option<String>,

    #[arg(long, env = "MAX_CLOCK_DRIFT_SECONDS", default_value = "60")]
    pub max_clock_drift_seconds: u64,

    #[arg(long, env = "HTTP_PORT", default_value = "8080")]
    pub http_port: u16,

    #[arg(long, env = "HTTP_HOST", default_value = "0.0.0.0")]
    pub http_host: String,

    #[arg(long, env = "API_KEY")]
    pub api_key: Option<String>,

    #[arg(long, env = "DATABASE_PATH", default_value = "validator.db")]
    pub database_path: String,

    #[arg(long, env = "GCS_BUCKET")]
    pub gcs_bucket: Option<String>,

    #[arg(long, env = "SCHEMA_CACHE_TTL_SECONDS", default_value = "3600", value_parser = parse_duration_seconds)]
    #[serde(with = "humantime_serde")]
    pub schema_cache_ttl: Duration,

    #[arg(long, env = "LOG_FORMAT", default_value = "pretty")]
    pub log_format: LogFormat,

    /// Enable price oracle invariants (metadata consistency and divergence checks)
    #[arg(long, env = "ENABLE_PRICE_ORACLE_INVARIANTS", default_value = "false")]
    pub enable_price_oracle_invariants: bool,

    /// Maximum allowed price divergence in basis points (100 bps = 1%)
    /// Only applies when price oracle invariants are enabled
    #[arg(long, env = "PRICE_DIVERGENCE_MAX_BPS", default_value = "100")]
    pub price_divergence_max_bps: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    #[default]
    Pretty,
    Json,
}

impl ValidatorConfig {
    pub const fn max_clock_drift(&self) -> Duration {
        Duration::from_secs(self.max_clock_drift_seconds)
    }
}

#[cfg(test)]
impl ValidatorConfig {
    pub fn test_config() -> Self {
        Self {
            mode: ValidatorMode::Primary,
            bridge_address: Address::ZERO,
            bridge_chain_id: 1,
            rpc_url: "http://localhost:8545".to_string(),
            ws_url: None,
            private_key: Some(
                "0x0000000000000000000000000000000000000000000000000000000000000001".to_string(),
            ),
            max_clock_drift_seconds: 60,
            http_port: 8080,
            http_host: "127.0.0.1".to_string(),
            api_key: Some("test-api-key".to_string()),
            database_path: ":memory:".to_string(),
            gcs_bucket: None,
            schema_cache_ttl: Duration::from_secs(3600),
            log_format: LogFormat::Pretty,
            enable_price_oracle_invariants: false,
            price_divergence_max_bps: 100,
        }
    }
}
