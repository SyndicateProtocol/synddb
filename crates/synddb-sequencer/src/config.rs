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
    ///
    /// WARNING: Data is lost after each request. Only use for testing.
    None,
    /// Local storage for testing with HTTP fetch API (CBOR format with COSE signatures)
    ///
    /// Uses `SQLite` for file persistence if `LOCAL_STORAGE_PATH` is set,
    /// otherwise uses in-memory storage.
    #[default]
    Local,
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
    #[arg(long, env = "PUBLISHER_TYPE", value_enum, default_value = "local")]
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

    /// Enable OpenTelemetry distributed tracing (requires otel feature)
    ///
    /// When enabled, traces are exported to the OTLP endpoint specified by
    /// `OTEL_EXPORTER_OTLP_ENDPOINT` (default: `http://localhost:4317`).
    #[arg(long, env = "OTEL_ENABLED", default_value = "false")]
    pub otel_enabled: bool,

    /// GCP project ID for Cloud Trace log correlation
    ///
    /// When set along with `otel_enabled`, logs are formatted for Cloud Logging
    /// with automatic trace correlation (logging.googleapis.com/trace fields).
    #[arg(long, env = "GCP_PROJECT_ID")]
    pub gcp_project_id: Option<String>,

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

    // ========================================================================
    // Outbound message monitor configuration
    // ========================================================================
    /// Path to the application's `SQLite` database for outbound message monitoring
    ///
    /// When set, the sequencer will poll this database for pending outbound
    /// messages in the `message_log` table and track their status.
    #[arg(long, env = "APP_DATABASE_PATH")]
    pub app_database_path: Option<String>,

    /// How often to poll for new outbound messages (in milliseconds)
    #[arg(long, env = "OUTBOUND_POLL_INTERVAL_MS", default_value = "1000")]
    pub outbound_poll_interval_ms: u64,

    // ========================================================================
    // TEE Bootstrap configuration (requires 'tee' feature)
    // ========================================================================
    /// Enable TEE key bootstrapping with on-chain registration
    ///
    /// When enabled, the sequencer will:
    /// 1. Generate an ephemeral signing key
    /// 2. Fetch a TEE attestation token
    /// 3. Generate an SP1 proof via the proof service
    /// 4. Submit the proof to the `TeeKeyManager` contract
    /// 5. Wait for on-chain confirmation before accepting requests
    #[arg(long, env = "ENABLE_KEY_BOOTSTRAP", default_value = "false")]
    pub enable_key_bootstrap: bool,

    /// Bridge contract address for key registration
    #[arg(long, env = "BRIDGE_CONTRACT_ADDRESS")]
    pub bridge_contract_address: Option<String>,

    /// RPC URL for verifying key registration
    #[arg(long, env = "BOOTSTRAP_RPC_URL")]
    pub bootstrap_rpc_url: Option<String>,

    /// Chain ID for EIP-712 signatures
    #[arg(long, env = "BOOTSTRAP_CHAIN_ID")]
    pub bootstrap_chain_id: Option<u64>,

    /// Relayer URL for key registration (relayer pays gas)
    #[arg(long, env = "RELAYER_URL")]
    pub relayer_url: Option<String>,

    /// URL of the proof service for generating SP1 proofs
    #[arg(long, env = "PROOF_SERVICE_URL")]
    pub proof_service_url: Option<String>,

    /// Expected audience claim for attestation tokens
    #[arg(long, env = "ATTESTATION_AUDIENCE")]
    pub attestation_audience: Option<String>,

    /// Cosign signature over the container image digest (64 bytes r||s, hex-encoded with 0x prefix)
    ///
    /// This is the ECDSA P-256 signature produced by cosign when signing the container image.
    /// Required when `enable_key_bootstrap` is true.
    #[arg(long, env = "COSIGN_SIGNATURE")]
    pub cosign_signature: Option<String>,

    /// Cosign public key for signature verification (64 or 65 bytes, hex-encoded with 0x prefix)
    ///
    /// P-256 public key: either 64 bytes (x||y) or 65 bytes (0x04||x||y uncompressed).
    /// Required when `enable_key_bootstrap` is true.
    #[arg(long, env = "COSIGN_PUBKEY")]
    pub cosign_pubkey: Option<String>,

    /// Timeout for proof generation (default: 10 minutes)
    #[arg(long, env = "PROOF_TIMEOUT", default_value = "600s", value_parser = humantime::parse_duration)]
    #[serde(with = "humantime_serde")]
    pub proof_timeout: Duration,

    /// Total bootstrap timeout (default: 15 minutes)
    #[arg(long, env = "BOOTSTRAP_TIMEOUT", default_value = "900s", value_parser = humantime::parse_duration)]
    #[serde(with = "humantime_serde")]
    pub bootstrap_timeout: Duration,
}

impl SequencerConfig {
    /// Validate bootstrap configuration
    ///
    /// Returns an error if bootstrap is enabled but required fields are missing.
    pub fn validate_bootstrap_config(&self) -> Result<(), String> {
        if !self.enable_key_bootstrap {
            return Ok(());
        }

        if self.bridge_contract_address.is_none() {
            return Err(
                "BRIDGE_CONTRACT_ADDRESS is required when ENABLE_KEY_BOOTSTRAP=true".into(),
            );
        }
        if self.bootstrap_rpc_url.is_none() {
            return Err("BOOTSTRAP_RPC_URL is required when ENABLE_KEY_BOOTSTRAP=true".into());
        }
        if self.bootstrap_chain_id.is_none() {
            return Err("BOOTSTRAP_CHAIN_ID is required when ENABLE_KEY_BOOTSTRAP=true".into());
        }
        if self.relayer_url.is_none() {
            return Err("RELAYER_URL is required when ENABLE_KEY_BOOTSTRAP=true".into());
        }
        if self.proof_service_url.is_none() {
            return Err("PROOF_SERVICE_URL is required when ENABLE_KEY_BOOTSTRAP=true".into());
        }
        if self.attestation_audience.is_none() {
            return Err("ATTESTATION_AUDIENCE is required when ENABLE_KEY_BOOTSTRAP=true".into());
        }
        if self.cosign_signature.is_none() {
            return Err("COSIGN_SIGNATURE is required when ENABLE_KEY_BOOTSTRAP=true".into());
        }
        if self.cosign_pubkey.is_none() {
            return Err("COSIGN_PUBKEY is required when ENABLE_KEY_BOOTSTRAP=true".into());
        }

        Ok(())
    }

    /// Get batch configuration from sequencer config
    pub const fn batch_config(&self) -> BatchConfig {
        BatchConfig {
            max_messages: self.batch_max_messages,
            max_batch_bytes: self.batch_max_bytes,
            flush_interval: self.batch_flush_interval,
        }
    }

    /// Create config with defaults for testing
    pub fn for_testing() -> Self {
        Self::parse_from(["synddb-sequencer"])
    }

    // =========================================================================
    // Builder methods for testing
    // =========================================================================

    /// Set the bind address
    #[must_use]
    pub const fn with_bind_address(mut self, addr: SocketAddr) -> Self {
        self.bind_address = addr;
        self
    }

    /// Set the publisher type
    #[must_use]
    pub const fn with_publisher_type(mut self, publisher_type: PublisherType) -> Self {
        self.publisher_type = publisher_type;
        self
    }

    /// Set the local storage path for `SQLite` backend
    #[must_use]
    pub fn with_local_storage_path(mut self, path: impl Into<String>) -> Self {
        self.local_storage_path = Some(path.into());
        self
    }

    /// Set the GCS bucket
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

    /// Set the request timeout
    #[must_use]
    pub const fn with_request_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = timeout;
        self
    }

    /// Enable attestation verification
    #[must_use]
    pub const fn with_verify_attestation(mut self, verify: bool) -> Self {
        self.verify_attestation = verify;
        self
    }

    /// Set batch configuration
    #[must_use]
    pub const fn with_batch_config(mut self, max_messages: usize, max_bytes: usize) -> Self {
        self.batch_max_messages = max_messages;
        self.batch_max_bytes = max_bytes;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_defaults() {
        let config = SequencerConfig::for_testing();

        assert_eq!(
            config.bind_address,
            "0.0.0.0:8433".parse::<SocketAddr>().unwrap()
        );
        assert_eq!(config.request_timeout, Duration::from_secs(30));
        assert_eq!(config.max_message_size, 10_485_760);
        assert!(!config.verify_attestation);
    }
}
