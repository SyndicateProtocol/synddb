//! Configuration for the sequencer node
//!
//! Supports both CLI arguments and environment variables following the
//! same pattern as synddb-client.

use clap::Parser;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::time::Duration;

/// Sequencer node configuration
#[derive(Debug, Clone, Serialize, Deserialize, Parser)]
#[command(name = "synddb-sequencer")]
#[command(about = "SyndDB Sequencer - orders and signs messages from client applications")]
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

    /// GCS bucket for message persistence (enables GCS publisher)
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
