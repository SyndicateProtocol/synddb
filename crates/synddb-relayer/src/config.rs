//! Relayer configuration
//!
//! Supports two modes:
//! 1. Multi-tenant via TOML config file (set `RELAYER_CONFIG_PATH`)
//! 2. Single-application via environment variables (fallback)

use alloy::primitives::{Address, B256};
use clap::Parser;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, net::SocketAddr, path::PathBuf};

/// Per-application configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ApplicationConfig {
    /// Audience string that identifies this application
    pub audience: String,

    /// Allowed image digests for this application
    /// Only TEEs running these code versions will be registered
    pub allowed_image_digests: Vec<B256>,
}

/// TOML configuration file structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ConfigFile {
    /// RPC URL for transaction submission
    pub rpc_url: String,

    /// Chain ID for EIP-712 domain
    pub chain_id: u64,

    /// Bridge contract address (for key registration)
    pub bridge_address: Address,

    /// Relayer's private key (hex-encoded, for paying gas)
    pub private_key: String,

    /// Listen address for HTTP server
    #[serde(default = "default_listen_addr")]
    pub listen_addr: SocketAddr,

    /// Per-application configurations
    #[serde(rename = "application")]
    pub applications: Vec<ApplicationConfig>,
}

fn default_listen_addr() -> SocketAddr {
    "0.0.0.0:8082".parse().unwrap()
}

/// CLI arguments (for env var fallback mode)
#[derive(Debug, Clone, Parser)]
pub(crate) struct CliConfig {
    /// Path to TOML configuration file (enables multi-tenant mode)
    #[arg(long, env = "RELAYER_CONFIG_PATH")]
    pub config_path: Option<PathBuf>,

    /// RPC URL for transaction submission
    #[arg(long, env = "RPC_URL")]
    pub rpc_url: Option<String>,

    /// Chain ID for EIP-712 domain
    #[arg(long, env = "CHAIN_ID")]
    pub chain_id: Option<u64>,

    /// Bridge contract address
    #[arg(long, env = "BRIDGE_CONTRACT_ADDRESS")]
    pub bridge_address: Option<Address>,

    /// Relayer's private key (hex-encoded, for paying gas)
    #[arg(long, env = "RELAYER_PRIVATE_KEY")]
    pub private_key: Option<String>,

    /// Listen address for HTTP server
    #[arg(long, env = "RELAYER_LISTEN_ADDR", default_value = "0.0.0.0:8082")]
    pub listen_addr: SocketAddr,

    /// Required audience string (single-app mode)
    #[arg(long, env = "REQUIRED_AUDIENCE")]
    pub required_audience: Option<String>,

    /// Allowed image digests (comma-separated hex hashes, single-app mode)
    #[arg(long, env = "ALLOWED_IMAGE_DIGESTS")]
    pub allowed_image_digests: Option<String>,
}

/// Runtime configuration for the relayer
#[derive(Debug, Clone)]
pub(crate) struct RelayerConfig {
    /// RPC URL for transaction submission
    pub rpc_url: String,

    /// Chain ID for EIP-712 domain
    pub chain_id: u64,

    /// Bridge contract address
    pub bridge_address: Address,

    /// Relayer's private key (hex-encoded, for paying gas)
    pub private_key: String,

    /// Listen address for HTTP server
    pub listen_addr: SocketAddr,

    /// Application configs indexed by audience hash
    pub applications: HashMap<B256, ApplicationConfig>,
}

impl RelayerConfig {
    /// Load configuration from CLI args / env vars
    ///
    /// If `RELAYER_CONFIG_PATH` is set, loads from TOML file.
    /// Otherwise, uses environment variables for single-app mode.
    pub(crate) fn load() -> anyhow::Result<Self> {
        let cli = CliConfig::parse();

        if let Some(config_path) = &cli.config_path {
            Self::from_file(config_path)
        } else {
            Self::from_env(cli)
        }
    }

    /// Load from TOML configuration file
    fn from_file(path: &PathBuf) -> anyhow::Result<Self> {
        let contents = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("Failed to read config file {:?}: {}", path, e))?;

        let config: ConfigFile = toml::from_str(&contents)
            .map_err(|e| anyhow::anyhow!("Failed to parse config file: {}", e))?;

        if config.applications.is_empty() {
            anyhow::bail!("Config file must contain at least one [[application]]");
        }

        let mut applications = HashMap::new();
        for app in config.applications {
            if app.allowed_image_digests.is_empty() {
                anyhow::bail!(
                    "Application {} must have at least one allowed_image_digest",
                    app.audience
                );
            }
            let audience_hash = compute_audience_hash(&app.audience);
            applications.insert(audience_hash, app);
        }

        Ok(Self {
            rpc_url: config.rpc_url,
            chain_id: config.chain_id,
            bridge_address: config.bridge_address,
            private_key: config.private_key,
            listen_addr: config.listen_addr,
            applications,
        })
    }

    /// Load from environment variables (single-app mode)
    fn from_env(cli: CliConfig) -> anyhow::Result<Self> {
        let rpc_url = cli
            .rpc_url
            .ok_or_else(|| anyhow::anyhow!("RPC_URL is required"))?;
        let chain_id = cli
            .chain_id
            .ok_or_else(|| anyhow::anyhow!("CHAIN_ID is required"))?;
        let bridge_address = cli
            .bridge_address
            .ok_or_else(|| anyhow::anyhow!("BRIDGE_CONTRACT_ADDRESS is required"))?;
        let private_key = cli
            .private_key
            .ok_or_else(|| anyhow::anyhow!("RELAYER_PRIVATE_KEY is required"))?;
        let allowed_digests_str = cli
            .allowed_image_digests
            .ok_or_else(|| anyhow::anyhow!("ALLOWED_IMAGE_DIGESTS is required"))?;
        let audience = cli
            .required_audience
            .ok_or_else(|| anyhow::anyhow!("REQUIRED_AUDIENCE is required"))?;

        // Parse allowed digests
        let allowed_image_digests: Vec<B256> = allowed_digests_str
            .split(',')
            .filter_map(|s| parse_b256(s.trim()))
            .collect();

        if allowed_image_digests.is_empty() {
            anyhow::bail!("ALLOWED_IMAGE_DIGESTS must contain at least one valid digest");
        }

        // Validate private key
        let key = private_key.trim_start_matches("0x");
        if key.len() != 64 || hex::decode(key).is_err() {
            anyhow::bail!("RELAYER_PRIVATE_KEY must be a 32-byte hex string");
        }

        let audience_hash = compute_audience_hash(&audience);
        let app_config = ApplicationConfig {
            audience,
            allowed_image_digests,
        };

        let mut applications = HashMap::new();
        applications.insert(audience_hash, app_config);

        Ok(Self {
            rpc_url,
            chain_id,
            bridge_address,
            private_key,
            listen_addr: cli.listen_addr,
            applications,
        })
    }

    /// Look up application config by audience hash
    pub(crate) fn get_application(&self, audience_hash: &B256) -> Option<&ApplicationConfig> {
        self.applications.get(audience_hash)
    }

    /// Check if an image digest is allowed for a given audience
    pub(crate) fn is_digest_allowed(&self, audience_hash: &B256, image_digest: &B256) -> bool {
        self.applications
            .get(audience_hash)
            .is_some_and(|app| app.allowed_image_digests.contains(image_digest))
    }
}

/// Compute audience hash from string
pub(crate) fn compute_audience_hash(audience: &str) -> B256 {
    use alloy::primitives::keccak256;
    keccak256(audience.as_bytes())
}

/// Parse a hex string into B256
fn parse_b256(s: &str) -> Option<B256> {
    let s = s.trim().trim_start_matches("0x");
    let bytes = hex::decode(s).ok()?;
    (bytes.len() == 32).then(|| B256::from_slice(&bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_b256() {
        let hash = format!("0x{}", "ab".repeat(32));
        let parsed = parse_b256(&hash);
        assert!(parsed.is_some());
        assert_eq!(parsed.unwrap(), B256::from([0xab; 32]));
    }

    #[test]
    fn test_parse_b256_no_prefix() {
        let hash = "cd".repeat(32);
        let parsed = parse_b256(&hash);
        assert!(parsed.is_some());
        assert_eq!(parsed.unwrap(), B256::from([0xcd; 32]));
    }

    #[test]
    fn test_parse_b256_invalid() {
        assert!(parse_b256("0x123").is_none());
        assert!(parse_b256("not_hex").is_none());
    }

    #[test]
    fn test_compute_audience_hash() {
        let hash1 = compute_audience_hash("https://my-app.example.com");
        let hash2 = compute_audience_hash("https://my-app.example.com");
        let hash3 = compute_audience_hash("https://other-app.example.com");

        assert_eq!(hash1, hash2);
        assert_ne!(hash1, hash3);
    }

    #[test]
    fn test_config_file_parse() {
        let toml = r#"
            rpc_url = "http://localhost:8545"
            chain_id = 31337
            bridge_address = "0x9fE46736679d2D9a65F0992F2272dE9f3c7fa6e0"
            private_key = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"

            [[application]]
            audience = "https://sequencer.example.com"
            allowed_image_digests = [
                "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "0xcccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
            ]

            [[application]]
            audience = "https://validator.example.com"
            allowed_image_digests = [
                "0xeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"
            ]
        "#;

        let config: ConfigFile = toml::from_str(toml).unwrap();
        assert_eq!(config.applications.len(), 2);
        assert_eq!(config.applications[0].allowed_image_digests.len(), 2);
        assert_eq!(config.applications[1].allowed_image_digests.len(), 1);
    }
}
