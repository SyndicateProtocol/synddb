//! Configuration for the validator

use clap::{Parser, ValueEnum};
use serde::{Deserialize, Serialize};
use std::{fmt, net::SocketAddr, time::Duration};
use strum::{EnumIter, IntoEnumIterator};

/// Well-known addresses for local Anvil development.
///
/// These addresses are deterministic when deploying with `./scripts/deploy-local.sh`.
/// Only use these for local development - production deployments must set real addresses.
pub mod local_defaults {
    /// Anvil chain ID
    pub const CHAIN_ID: u64 = 31337;

    /// Bridge contract address (deployed at nonce 1 from Anvil account 0)
    pub const BRIDGE_CONTRACT: &str = "0xe7f1725E7734CE288F8367e1Bb143E90bb3F0512";

    /// `MockWETH` address (deployed at nonce 0 from Anvil account 0)
    pub const WETH: &str = "0x5FbDB2315678afecb367f032d93F642f64180aa3";

    /// `PriceOracle` address (deployed at nonce 3 from Anvil account 0)
    pub const PRICE_ORACLE: &str = "0xCf7Ed3AccA5a467e9e704C703E8D87F634fB0Fc9";

    /// Sequencer public key (corresponds to Anvil account 0 private key)
    pub const SEQUENCER_PUBKEY: &str = "8318535b54105d4a7aae60c08fc45f9687181b4fdfc625bd1a753fa7397fed753547f11ca8696646f2f3acb08e31016afac23e630c5d11f59f61fef57b0d2aa5";
}

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
    ///
    /// The audit trail is always enabled - it's critical for production reliability
    /// as it allows validators to continue syncing during schema changes instead of
    /// halting until manual intervention.
    #[arg(
        long,
        env = "PENDING_CHANGESETS_DB_PATH",
        default_value = "/data/pending_changesets.db"
    )]
    pub pending_changesets_db_path: String,

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
    /// When chain ID is 31337 (Anvil), uses local development defaults for missing values.
    /// Note: signing key is generated automatically inside the TEE, not provided via config.
    pub fn validate_bridge_config(&self) -> Result<(), String> {
        if !self.bridge_signer {
            return Ok(());
        }

        let is_local = self.bridge_chain_id == Some(local_defaults::CHAIN_ID);

        if self.bridge_contract.is_none() && !is_local {
            return Err("--bridge-contract is required when --bridge-signer is enabled".into());
        }

        if self.bridge_chain_id.is_none() {
            return Err("--bridge-chain-id is required when --bridge-signer is enabled".into());
        }

        Ok(())
    }

    /// Get bridge contract address, using local default for Anvil (chain ID 31337)
    ///
    /// Returns `None` if `bridge_contract` is not set and not running on Anvil.
    pub fn bridge_contract_with_local_fallback(&self) -> Option<String> {
        self.bridge_contract.clone().or_else(|| {
            (self.bridge_chain_id == Some(local_defaults::CHAIN_ID))
                .then(|| local_defaults::BRIDGE_CONTRACT.to_string())
        })
    }

    // =========================================================================
    // Builder methods for testing
    // =========================================================================

    /// Set the database path
    #[must_use]
    pub fn with_database_path(mut self, path: impl Into<String>) -> Self {
        self.database_path = path.into();
        self
    }

    /// Set the state database path
    #[must_use]
    pub fn with_state_db_path(mut self, path: impl Into<String>) -> Self {
        self.state_db_path = path.into();
        self
    }

    /// Set the fetcher type
    #[must_use]
    pub const fn with_fetcher_type(mut self, fetcher_type: FetcherType) -> Self {
        self.fetcher_type = fetcher_type;
        self
    }

    /// Set the sequencer URL for HTTP fetcher
    #[must_use]
    pub fn with_sequencer_url(mut self, url: impl Into<String>) -> Self {
        self.sequencer_url = Some(url.into());
        self
    }

    /// Set the GCS bucket for GCS fetcher
    #[must_use]
    pub fn with_gcs_bucket(mut self, bucket: impl Into<String>) -> Self {
        self.gcs_bucket = Some(bucket.into());
        self
    }

    /// Set the GCS prefix
    #[must_use]
    pub fn with_gcs_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.gcs_prefix = prefix.into();
        self
    }

    /// Set the GCS emulator host for testing
    #[must_use]
    pub fn with_gcs_emulator_host(mut self, host: impl Into<String>) -> Self {
        self.gcs_storage_emulator_host = Some(host.into());
        self
    }

    /// Set the bind address for HTTP API
    #[must_use]
    pub const fn with_bind_address(mut self, addr: SocketAddr) -> Self {
        self.bind_address = addr;
        self
    }

    /// Set the sync poll interval
    #[must_use]
    pub const fn with_sync_interval(mut self, interval: Duration) -> Self {
        self.sync_interval = interval;
        self
    }

    /// Set the start sequence number
    #[must_use]
    pub const fn with_start_sequence(mut self, sequence: u64) -> Self {
        self.start_sequence = sequence;
        self
    }

    /// Enable or disable batch sync mode
    #[must_use]
    pub const fn with_batch_sync_enabled(mut self, enabled: bool) -> Self {
        self.batch_sync_enabled = enabled;
        self
    }

    /// Configure bridge signer mode
    ///
    /// Note: signing key is generated automatically inside the TEE.
    #[must_use]
    pub fn with_bridge_signer(mut self, contract: impl Into<String>, chain_id: u64) -> Self {
        self.bridge_signer = true;
        self.bridge_contract = Some(contract.into());
        self.bridge_chain_id = Some(chain_id);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 64-byte uncompressed public key corresponding to test private key
    // ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80
    const TEST_PUBKEY: &str = "8318535b54105d4a7aae60c08fc45f9687181b4fdfc625bd1a753fa7397fed753547f11ca8696646f2f3acb08e31016afac23e630c5d11f59f61fef57b0d2aa5";

    /// Clear all bridge-related env vars that might be set by .env.defaults
    fn clear_bridge_env_vars() {
        std::env::remove_var("BRIDGE_SIGNER");
        std::env::remove_var("BRIDGE_CONTRACT");
        std::env::remove_var("BRIDGE_CHAIN_ID");
        std::env::remove_var("BRIDGE_SIGNATURE_ENDPOINT");
    }

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
        ]);

        assert!(config.bridge_signer);
        assert_eq!(
            config.bridge_contract,
            Some("0x1234567890abcdef1234567890abcdef12345678".to_string())
        );
        assert_eq!(config.bridge_chain_id, Some(1));
        assert!(config.validate_bridge_config().is_ok());
    }

    #[test]
    fn test_bridge_signer_validation_missing_contract() {
        // Clear env vars that might be set by .env.defaults
        clear_bridge_env_vars();

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

    #[test]
    fn test_bridge_contract_local_fallback_anvil() {
        // When chain ID is 31337, bridge contract should use local default if not set
        let config = ValidatorConfig::parse_from([
            "synddb-validator",
            "--sequencer-pubkey",
            TEST_PUBKEY,
            "--bridge-signer",
            "--bridge-chain-id",
            "31337",
        ]);

        assert_eq!(
            config.bridge_contract_with_local_fallback(),
            Some(local_defaults::BRIDGE_CONTRACT.to_string())
        );
        // Validation should pass for Anvil chain without explicit bridge-contract
        assert!(config.validate_bridge_config().is_ok());
    }

    #[test]
    fn test_bridge_contract_local_fallback_non_anvil() {
        // Clear env vars that might be set by .env.defaults
        clear_bridge_env_vars();

        // When chain ID is NOT 31337, bridge contract should NOT use local default
        let config = ValidatorConfig::parse_from([
            "synddb-validator",
            "--sequencer-pubkey",
            TEST_PUBKEY,
            "--bridge-signer",
            "--bridge-chain-id",
            "1", // Mainnet
        ]);

        assert_eq!(config.bridge_contract_with_local_fallback(), None);
        // Validation should fail for non-Anvil chain without explicit bridge-contract
        assert!(config.validate_bridge_config().is_err());
    }

    #[test]
    fn test_bridge_contract_explicit_overrides_fallback() {
        // Explicitly set bridge contract should be used even on Anvil
        let config = ValidatorConfig::parse_from([
            "synddb-validator",
            "--sequencer-pubkey",
            TEST_PUBKEY,
            "--bridge-signer",
            "--bridge-chain-id",
            "31337",
            "--bridge-contract",
            "0x1111111111111111111111111111111111111111",
        ]);

        assert_eq!(
            config.bridge_contract_with_local_fallback(),
            Some("0x1111111111111111111111111111111111111111".to_string())
        );
    }
}
