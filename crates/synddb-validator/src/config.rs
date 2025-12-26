//! Configuration for the validator

use clap::{Parser, ValueEnum};
use serde::{Deserialize, Serialize};
use std::{fmt, net::SocketAddr, time::Duration};
use strum::{EnumIter, IntoEnumIterator};

/// Available fetcher types for retrieving messages from the storage layer
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ValueEnum, EnumIter,
)]
#[serde(rename_all = "lowercase")]
pub enum FetcherType {
    /// HTTP fetcher for sequencer's local storage API
    #[default]
    Http,
    /// Google Cloud Storage fetcher
    Gcs,
}

impl FetcherType {
    /// Get all supported fetcher types as strings
    pub fn supported_types() -> Vec<String> {
        Self::iter().map(|t| t.to_string()).collect()
    }
}

impl fmt::Display for FetcherType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http => write!(f, "http"),
            Self::Gcs => write!(f, "gcs"),
        }
    }
}

/// `SyndDB` Validator configuration
#[derive(Debug, Clone, Serialize, Deserialize, Parser)]
#[command(name = "synddb-validator")]
#[command(about = "SyndDB Validator - validates sequencer messages and applies changesets")]
pub struct ValidatorConfig {
    /// Path to the `SQLite` database file for replicated state
    #[arg(long, env = "DATABASE_PATH", default_value = "/data/validator.db")]
    pub database_path: String,

    /// Path to the `SQLite` database file for validator state (sequences, etc.)
    #[arg(
        long,
        env = "STATE_DB_PATH",
        default_value = "/data/validator_state.db"
    )]
    pub state_db_path: String,

    /// Path to the `SQLite` database file for pending changesets (audit trail)
    ///
    /// Stores changesets that couldn't be applied due to schema mismatch.
    /// These are verified when a snapshot arrives to maintain audit trail.
    #[arg(
        long,
        env = "PENDING_CHANGESETS_DB_PATH",
        default_value = "/data/pending_changesets.db"
    )]
    pub pending_changesets_db_path: String,

    /// Enable audit trail for schema mismatch recovery
    ///
    /// When enabled, changesets that fail due to schema mismatch are stored
    /// and verified against snapshots for auditability.
    #[arg(long, env = "AUDIT_TRAIL_ENABLED", default_value = "true", action = clap::ArgAction::Set)]
    pub audit_trail_enabled: bool,

    /// Expected sequencer public key (for signature verification)
    ///
    /// This should be the 64-byte uncompressed public key in hex format (128 hex chars),
    /// with optional "0x" prefix. The sequencer logs its public key at startup.
    #[arg(long, env = "SEQUENCER_PUBKEY")]
    pub sequencer_pubkey: String,

    /// Fetcher type for retrieving messages from storage layer
    #[arg(long, env = "FETCHER_TYPE", value_enum, default_value = "http")]
    pub fetcher_type: FetcherType,

    /// Sequencer URL for fetching messages via HTTP (required when `fetcher_type=http`)
    #[arg(long, env = "SEQUENCER_URL")]
    pub sequencer_url: Option<String>,

    /// GCS bucket for fetching messages (required when `fetcher_type=gcs`)
    #[arg(long, env = "GCS_BUCKET")]
    pub gcs_bucket: Option<String>,

    /// GCS path prefix (must match sequencer)
    #[arg(long, env = "GCS_PREFIX", default_value = "sequencer")]
    pub gcs_prefix: String,

    /// GCS emulator host URL for local testing.
    ///
    /// This is analogous to the standard `STORAGE_EMULATOR_HOST` environment variable
    /// recognized by Google Cloud client libraries. See <https://docs.cloud.google.com/go/docs/reference/cloud.google.com/go/storage/latest#hdr-Creating_a_Client>
    ///
    /// When set, the client uses anonymous
    /// authentication and connects to the specified emulator (e.g., `fake-gcs-server`)
    /// instead of real GCS.
    ///
    /// Example: `http://localhost:4443` or `http://fake-gcs:4443` in Docker.
    #[arg(long, env = "GCS_STORAGE_EMULATOR_HOST")]
    pub gcs_storage_emulator_host: Option<String>,

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

    // === Bridge Signer Mode ===
    /// Enable bridge signer mode - signs withdrawal messages for bridge contract
    #[arg(long, env = "BRIDGE_SIGNER")]
    pub bridge_signer: bool,

    /// Bridge contract address (required if --bridge-signer)
    #[arg(long, env = "BRIDGE_CONTRACT")]
    pub bridge_contract: Option<String>,

    /// Chain ID for the bridge contract (required if --bridge-signer)
    #[arg(long, env = "BRIDGE_CHAIN_ID")]
    pub bridge_chain_id: Option<u64>,

    /// Signing key for bridge operations (hex private key, required if --bridge-signer)
    #[arg(long, env = "BRIDGE_SIGNING_KEY")]
    pub bridge_signing_key: Option<String>,

    /// Endpoint to serve signatures for relayers
    #[arg(
        long,
        env = "BRIDGE_SIGNATURE_ENDPOINT",
        default_value = "0.0.0.0:8081"
    )]
    pub bridge_signature_endpoint: SocketAddr,

    // === Gap Detection ===
    /// Maximum number of retries when a sequence gap is detected
    #[arg(long, env = "GAP_RETRY_COUNT", default_value = "5")]
    pub gap_retry_count: u32,

    /// Delay between gap retry attempts
    #[arg(long, env = "GAP_RETRY_DELAY", default_value = "5s", value_parser = humantime::parse_duration)]
    #[serde(with = "humantime_serde")]
    pub gap_retry_delay: Duration,

    /// Skip gaps after max retries instead of erroring (use with caution)
    #[arg(long, env = "GAP_SKIP_ON_FAILURE", default_value = "false")]
    pub gap_skip_on_failure: bool,

    // === Batch Sync ===
    /// Enable batch sync mode when the fetcher supports it (default: true)
    #[arg(long, env = "BATCH_SYNC_ENABLED", default_value = "true", action = clap::ArgAction::Set)]
    pub batch_sync_enabled: bool,

    /// How often to refresh the batch index when syncing in batch mode
    #[arg(long, env = "BATCH_INDEX_REFRESH_INTERVAL", default_value = "10s", value_parser = humantime::parse_duration)]
    #[serde(with = "humantime_serde")]
    pub batch_index_refresh_interval: Duration,
}

impl ValidatorConfig {
    /// Create a config for testing with a specific sequencer public key
    pub fn with_sequencer_pubkey(pubkey: &str) -> Self {
        Self::parse_from([
            "synddb-validator",
            "--sequencer-pubkey",
            pubkey,
            "--database-path",
            ":memory:",
            "--state-db-path",
            ":memory:",
            "--pending-changesets-db-path",
            ":memory:",
        ])
    }

    /// Check if bridge signer mode is enabled
    pub const fn is_bridge_signer(&self) -> bool {
        self.bridge_signer
    }

    /// Validate bridge signer configuration
    ///
    /// Returns an error if bridge signer is enabled but required fields are missing.
    pub fn validate_bridge_config(&self) -> Result<(), String> {
        if !self.bridge_signer {
            return Ok(());
        }

        if self.bridge_contract.is_none() {
            return Err("--bridge-contract is required when --bridge-signer is enabled".into());
        }

        if self.bridge_chain_id.is_none() {
            return Err("--bridge-chain-id is required when --bridge-signer is enabled".into());
        }

        if self.bridge_signing_key.is_none() {
            return Err("--bridge-signing-key is required when --bridge-signer is enabled".into());
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 64-byte uncompressed public key corresponding to test private key
    // ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80
    const TEST_PUBKEY: &str = "8318535b54105d4a7aae60c08fc45f9687181b4fdfc625bd1a753fa7397fed753547f11ca8696646f2f3acb08e31016afac23e630c5d11f59f61fef57b0d2aa5";

    #[test]
    fn test_config_defaults() {
        let config =
            ValidatorConfig::parse_from(["synddb-validator", "--sequencer-pubkey", TEST_PUBKEY]);

        assert_eq!(config.database_path, "/data/validator.db");
        assert_eq!(config.gcs_prefix, "sequencer");
        assert_eq!(config.sync_interval, Duration::from_secs(1));
        assert_eq!(config.start_sequence, 0);
        assert!(!config.log_json);
        assert!(!config.bridge_signer);
        // Batch sync defaults
        assert!(config.batch_sync_enabled);
        assert_eq!(config.batch_index_refresh_interval, Duration::from_secs(10));
    }

    #[test]
    fn test_config_with_gcs() {
        let config = ValidatorConfig::parse_from([
            "synddb-validator",
            "--sequencer-pubkey",
            TEST_PUBKEY,
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
        let config = ValidatorConfig::with_sequencer_pubkey(TEST_PUBKEY);

        assert_eq!(config.sequencer_pubkey, TEST_PUBKEY);
        assert_eq!(config.database_path, ":memory:");
    }

    #[test]
    fn test_bridge_signer_config() {
        let config = ValidatorConfig::parse_from([
            "synddb-validator",
            "--sequencer-pubkey",
            "8318535b54105d4a7aae60c08fc45f9687181b4fdfc625bd1a753fa7397fed753547f11ca8696646f2f3acb08e31016afac23e630c5d11f59f61fef57b0d2aa5",
            "--bridge-signer",
            "--bridge-contract",
            "0x1234567890abcdef1234567890abcdef12345678",
            "--bridge-chain-id",
            "1",
            "--bridge-signing-key",
            "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        ]);

        assert!(config.bridge_signer);
        assert_eq!(
            config.bridge_contract,
            Some("0x1234567890abcdef1234567890abcdef12345678".to_string())
        );
        assert_eq!(config.bridge_chain_id, Some(1));
        assert!(config.bridge_signing_key.is_some());
        assert!(config.validate_bridge_config().is_ok());
    }

    #[test]
    fn test_bridge_signer_validation_missing_contract() {
        let config = ValidatorConfig::parse_from([
            "synddb-validator",
            "--sequencer-pubkey",
            "8318535b54105d4a7aae60c08fc45f9687181b4fdfc625bd1a753fa7397fed753547f11ca8696646f2f3acb08e31016afac23e630c5d11f59f61fef57b0d2aa5",
            "--bridge-signer",
        ]);

        assert!(config.validate_bridge_config().is_err());
    }

    #[test]
    fn test_bridge_signer_disabled_no_validation() {
        let config = ValidatorConfig::parse_from([
            "synddb-validator",
            "--sequencer-pubkey",
            "8318535b54105d4a7aae60c08fc45f9687181b4fdfc625bd1a753fa7397fed753547f11ca8696646f2f3acb08e31016afac23e630c5d11f59f61fef57b0d2aa5",
        ]);

        assert!(config.validate_bridge_config().is_ok());
    }

    #[test]
    fn test_batch_sync_config() {
        let config = ValidatorConfig::parse_from([
            "synddb-validator",
            "--sequencer-pubkey",
            "8318535b54105d4a7aae60c08fc45f9687181b4fdfc625bd1a753fa7397fed753547f11ca8696646f2f3acb08e31016afac23e630c5d11f59f61fef57b0d2aa5",
            "--batch-sync-enabled",
            "false",
            "--batch-index-refresh-interval",
            "30s",
        ]);

        assert!(!config.batch_sync_enabled);
        assert_eq!(config.batch_index_refresh_interval, Duration::from_secs(30));
    }
}
