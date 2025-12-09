//! Configuration for the sequencer node
//!
//! Supports both CLI arguments and environment variables following the
//! same pattern as synddb-client.

use clap::{Parser, ValueEnum};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::net::SocketAddr;
use std::time::Duration;
use strum::{EnumIter, IntoEnumIterator};

/// Available publisher types for message persistence
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ValueEnum, EnumIter)]
#[serde(rename_all = "lowercase")]
pub enum PublisherType {
    /// No persistence (messages only kept in memory during request)
    #[default]
    None,
    /// Local `SQLite` storage with HTTP fetch API
    Local,
    /// Google Cloud Storage
    Gcs,
}

impl PublisherType {
    /// Get all supported publisher types as strings
    pub fn supported_types() -> Vec<String> {
        Self::iter().map(|t| t.to_string()).collect()
    }
}

impl fmt::Display for PublisherType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None => write!(f, "none"),
            Self::Local => write!(f, "local"),
            Self::Gcs => write!(f, "gcs"),
        }
    }
}

/// Sequencer node configuration
#[derive(Debug, Clone, Serialize, Deserialize, Parser)]
#[command(name = "synddb-sequencer")]
#[command(
    about = "SyndDB Sequencer - HTTP server that receives changesets from client libraries and publishes to DA layers"
)]
pub struct SequencerConfig {
    /// HTTP server bind address
    #[arg(long, env = "BIND_ADDRESS", default_value = "0.0.0.0:8433")]
    pub bind_address: SocketAddr,

    /// Private key for signing messages (hex-encoded, without 0x prefix)
    ///
    /// This key is used to sign all sequenced messages. The corresponding
    /// public address can be verified by clients and on-chain contracts.
    #[arg(long, env = "SIGNING_KEY")]
    pub signing_key: String,

    /// Request timeout for HTTP operations
    #[arg(long, env = "REQUEST_TIMEOUT", default_value = "30s", value_parser = humantime::parse_duration)]
    #[serde(with = "humantime_serde")]
    pub request_timeout: Duration,

    /// Maximum message size in bytes (default: 10MB)
    #[arg(long, env = "MAX_MESSAGE_SIZE", default_value = "10485760")]
    pub max_message_size: usize,

    /// Enable TEE attestation verification for incoming requests
    #[arg(long, env = "VERIFY_ATTESTATION", default_value = "false")]
    pub verify_attestation: bool,

    /// Publisher type for message persistence
    #[arg(long, env = "PUBLISHER_TYPE", value_enum, default_value = "none")]
    pub publisher_type: PublisherType,

    /// Local `SQLite` database path (required when `publisher_type=local`)
    ///
    /// Use `:memory:` for in-memory storage (ephemeral, for testing).
    #[arg(long, env = "LOCAL_STORAGE_PATH")]
    pub local_storage_path: Option<String>,

    /// GCS bucket name (required when `publisher_type=gcs`)
    #[arg(long, env = "GCS_BUCKET")]
    pub gcs_bucket: Option<String>,

    /// GCS path prefix for messages
    #[arg(long, env = "GCS_PREFIX", default_value = "sequencer")]
    pub gcs_prefix: String,

    /// Output logs in JSON format (for production log aggregation)
    #[arg(long, env = "RUST_LOG_JSON", default_value = "false")]
    pub log_json: bool,

    /// Attestation service URL for TEE token verification
    #[arg(long, env = "ATTESTATION_SERVICE_URL")]
    pub attestation_service_url: Option<String>,

    /// Graceful shutdown timeout
    #[arg(long, env = "SHUTDOWN_TIMEOUT", default_value = "30s", value_parser = humantime::parse_duration)]
    #[serde(with = "humantime_serde")]
    pub shutdown_timeout: Duration,
}

impl SequencerConfig {
    /// Create config with defaults for testing
    ///
    /// Uses `parse_from` with a dummy signing key to get clap defaults,
    /// then replaces with the provided key.
    pub fn with_signing_key(signing_key: String) -> Self {
        // We need to provide the required --signing-key arg
        let mut config = Self::parse_from([
            "synddb-sequencer",
            "--signing-key",
            "0000000000000000000000000000000000000000000000000000000000000001",
        ]);
        config.signing_key = signing_key;
        config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_defaults() {
        let config = SequencerConfig::with_signing_key("a".repeat(64));

        assert_eq!(
            config.bind_address,
            "0.0.0.0:8433".parse::<SocketAddr>().unwrap()
        );
        assert_eq!(config.request_timeout, Duration::from_secs(30));
        assert_eq!(config.max_message_size, 10_485_760);
        assert!(!config.verify_attestation);
    }
}
