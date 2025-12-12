//! Configuration for the sequencer node
//!
//! Supports both CLI arguments and environment variables following the
//! same pattern as synddb-client.

use clap::{Parser, ValueEnum};
use serde::{Deserialize, Serialize};
use std::{fmt, net::SocketAddr, time::Duration};
use strum::{EnumIter, IntoEnumIterator};

/// Configuration for message batching
///
/// Controls how messages are accumulated before publishing to storage.
/// A batch is flushed when any threshold is reached.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchConfig {
    /// Maximum number of messages per batch before flushing
    ///
    /// Default: 50 messages
    pub max_messages: usize,

    /// Maximum total size of messages in a batch (in bytes) before flushing
    ///
    /// This is the uncompressed payload size. The actual stored size will
    /// be smaller due to CBOR encoding and zstd compression.
    ///
    /// Default: 1MB (1,048,576 bytes)
    pub max_batch_bytes: usize,

    /// Maximum time to wait before flushing a partial batch
    ///
    /// This ensures messages are published even during low-traffic periods.
    ///
    /// Default: 5 seconds
    #[serde(with = "humantime_serde")]
    pub flush_interval: Duration,
}

impl Default for BatchConfig {
    fn default() -> Self {
        Self {
            max_messages: 50,
            max_batch_bytes: 1_048_576, // 1MB
            flush_interval: Duration::from_secs(5),
        }
    }
}

/// Available publisher types for message persistence
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ValueEnum, EnumIter,
)]
#[serde(rename_all = "lowercase")]
pub enum PublisherType {
    /// No persistence (messages only kept in memory during request)
    #[default]
    None,
    /// Local `SQLite` storage with HTTP fetch API (JSON format, legacy)
    Local,
    /// Local in-memory storage with HTTP fetch API (CBOR format with COSE signatures)
    LocalCbor,
    /// Google Cloud Storage (CBOR format with COSE signatures)
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
            Self::LocalCbor => write!(f, "local-cbor"),
            Self::Gcs => write!(f, "gcs"),
        }
    }
}

/// Sequencer node configuration
#[derive(Debug, Clone, Serialize, Deserialize, Parser)]
#[command(name = "synddb-sequencer")]
#[command(
    about = "SyndDB Sequencer - HTTP server that receives changesets from client libraries and publishes to storage layers"
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

    // ========================================================================
    // Batching configuration (for CBOR file transport)
    // ========================================================================
    /// Maximum messages per batch before flushing to storage
    #[arg(long, env = "BATCH_MAX_MESSAGES", default_value = "50")]
    pub batch_max_messages: usize,

    /// Maximum batch size in bytes before flushing (uncompressed)
    #[arg(long, env = "BATCH_MAX_BYTES", default_value = "1048576")]
    pub batch_max_bytes: usize,

    /// Maximum time to wait before flushing a partial batch
    #[arg(long, env = "BATCH_FLUSH_INTERVAL", default_value = "5s", value_parser = humantime::parse_duration)]
    #[serde(with = "humantime_serde")]
    pub batch_flush_interval: Duration,
}

impl SequencerConfig {
    /// Get batch configuration from sequencer config
    pub const fn batch_config(&self) -> BatchConfig {
        BatchConfig {
            max_messages: self.batch_max_messages,
            max_batch_bytes: self.batch_max_bytes,
            flush_interval: self.batch_flush_interval,
        }
    }

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
