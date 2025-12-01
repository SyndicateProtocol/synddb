//! Configuration for the validator

use clap::Parser;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::time::Duration;

/// SyndDB Validator configuration
#[derive(Debug, Clone, Serialize, Deserialize, Parser)]
#[command(name = "synddb-validator")]
#[command(about = "SyndDB Validator - validates sequencer messages and applies changesets")]
pub struct ValidatorConfig {
    /// Path to the SQLite database file for replicated state
    #[arg(long, env = "DATABASE_PATH", default_value = "/data/validator.db")]
    pub database_path: String,

    /// Path to the SQLite database file for validator state (sequences, etc.)
    #[arg(
        long,
        env = "STATE_DB_PATH",
        default_value = "/data/validator_state.db"
    )]
    pub state_db_path: String,

    /// Expected sequencer address (for signature verification)
    #[arg(long, env = "SEQUENCER_ADDRESS")]
    pub sequencer_address: String,

    /// GCS bucket for fetching messages
    #[arg(long, env = "GCS_BUCKET")]
    pub gcs_bucket: Option<String>,

    /// GCS path prefix (must match sequencer)
    #[arg(long, env = "GCS_PREFIX", default_value = "sequencer")]
    pub gcs_prefix: String,

    /// HTTP API bind address
    #[arg(long, env = "BIND_ADDRESS", default_value = "0.0.0.0:8080")]
    pub bind_address: SocketAddr,

    /// Sync poll interval
    #[arg(long, env = "SYNC_INTERVAL", default_value = "1s", value_parser = humantime::parse_duration)]
    #[serde(with = "humantime_serde")]
    pub sync_interval: Duration,

    /// Sequence number to start syncing from (0 means start from beginning)
    #[arg(long, env = "START_SEQUENCE", default_value = "0")]
    pub start_sequence: u64,

    /// Enable JSON log format (for production log aggregation)
    #[arg(long, env = "LOG_JSON", default_value = "false")]
    pub log_json: bool,

    /// Graceful shutdown timeout
    #[arg(long, env = "SHUTDOWN_TIMEOUT", default_value = "30s", value_parser = humantime::parse_duration)]
    #[serde(with = "humantime_serde")]
    pub shutdown_timeout: Duration,
}

impl ValidatorConfig {
    /// Create a config for testing with a specific sequencer address
    pub fn with_sequencer_address(address: &str) -> Self {
        Self::parse_from([
            "synddb-validator",
            "--sequencer-address",
            address,
            "--database-path",
            ":memory:",
            "--state-db-path",
            ":memory:",
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_defaults() {
        let config = ValidatorConfig::parse_from([
            "synddb-validator",
            "--sequencer-address",
            "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
        ]);

        assert_eq!(config.database_path, "/data/validator.db");
        assert_eq!(config.gcs_prefix, "sequencer");
        assert_eq!(config.sync_interval, Duration::from_secs(1));
        assert_eq!(config.start_sequence, 0);
        assert!(!config.log_json);
    }

    #[test]
    fn test_config_with_gcs() {
        let config = ValidatorConfig::parse_from([
            "synddb-validator",
            "--sequencer-address",
            "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
            "--gcs-bucket",
            "my-bucket",
            "--gcs-prefix",
            "custom/prefix",
        ]);

        assert_eq!(config.gcs_bucket, Some("my-bucket".to_string()));
        assert_eq!(config.gcs_prefix, "custom/prefix");
    }

    #[test]
    fn test_config_test_helper() {
        let config =
            ValidatorConfig::with_sequencer_address("0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266");

        assert_eq!(
            config.sequencer_address,
            "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"
        );
        assert_eq!(config.database_path, ":memory:");
    }
}
