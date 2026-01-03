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
    /// Audience hash that identifies this application
    /// (extracted from TEE attestation public values)
    pub audience_hash: B256,

    /// `GasTreasury` contract address for this application
    pub treasury_address: Address,

    /// Allowed image digests for this application
    /// Only TEEs running these code versions will be funded
    pub allowed_image_digests: Vec<B256>,

    /// Maximum total funding per image digest per day (in wei)
    /// Stored as string in TOML since TOML doesn't support u128
    #[serde(
        default = "default_max_funding_per_digest_daily",
        deserialize_with = "deserialize_u128_from_str_or_num"
    )]
    pub max_funding_per_digest_daily: u128,

    /// Maximum funding per address (in wei)
    /// Stored as string in TOML since TOML doesn't support u128
    #[serde(
        default = "default_max_funding_per_address",
        deserialize_with = "deserialize_u128_from_str_or_num"
    )]
    pub max_funding_per_address: u128,
}

/// Deserialize u128 from either a string or a number
fn deserialize_u128_from_str_or_num<'de, D>(deserializer: D) -> Result<u128, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::{self, Visitor};

    struct U128Visitor;

    impl<'de> Visitor<'de> for U128Visitor {
        type Value = u128;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a string or integer representing a u128")
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            value.parse().map_err(de::Error::custom)
        }

        fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(value as u128)
        }

        fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            if value >= 0 {
                Ok(value as u128)
            } else {
                Err(de::Error::custom("negative values not allowed"))
            }
        }
    }

    deserializer.deserialize_any(U128Visitor)
}

const fn default_max_funding_per_digest_daily() -> u128 {
    1_000_000_000_000_000_000 // 1 ETH
}

const fn default_max_funding_per_address() -> u128 {
    50_000_000_000_000_000 // 0.05 ETH
}

/// TOML configuration file structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ConfigFile {
    /// RPC URL for transaction submission
    pub rpc_url: String,

    /// Chain ID for EIP-712 domain
    pub chain_id: u64,

    /// `TeeKeyManager` contract address (shared across all applications)
    pub key_manager_address: Address,

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

    /// `TeeKeyManager` contract address
    #[arg(long, env = "TEE_KEY_MANAGER_CONTRACT_ADDRESS")]
    pub key_manager_address: Option<Address>,

    /// `GasTreasury` contract address (single-app mode)
    #[arg(long, env = "GAS_TREASURY_CONTRACT_ADDRESS")]
    pub treasury_address: Option<Address>,

    /// Relayer's private key (hex-encoded, for paying gas)
    #[arg(long, env = "RELAYER_PRIVATE_KEY")]
    pub private_key: Option<String>,

    /// Listen address for HTTP server
    #[arg(long, env = "RELAYER_LISTEN_ADDR", default_value = "0.0.0.0:8082")]
    pub listen_addr: SocketAddr,

    /// Required audience hash (single-app mode)
    #[arg(long, env = "REQUIRED_AUDIENCE_HASH")]
    pub required_audience_hash: Option<String>,

    /// Allowed image digests (comma-separated hex hashes, single-app mode)
    #[arg(long, env = "ALLOWED_IMAGE_DIGESTS")]
    pub allowed_image_digests: Option<String>,

    /// Maximum total funding per image digest per day (in wei)
    #[arg(
        long,
        env = "MAX_FUNDING_PER_DIGEST_DAILY",
        default_value = "1000000000000000000"
    )]
    pub max_funding_per_digest_daily: u128,

    /// Maximum funding per address (in wei)
    #[arg(
        long,
        env = "MAX_FUNDING_PER_ADDRESS",
        default_value = "50000000000000000"
    )]
    pub max_funding_per_address: u128,
}

/// Runtime configuration for the relayer
#[derive(Debug, Clone)]
pub(crate) struct RelayerConfig {
    /// RPC URL for transaction submission
    pub rpc_url: String,

    /// Chain ID for EIP-712 domain
    pub chain_id: u64,

    /// `TeeKeyManager` contract address
    pub key_manager_address: Address,

    /// Relayer's private key (hex-encoded, for paying gas)
    pub private_key: String,

    /// Listen address for HTTP server
    pub listen_addr: SocketAddr,

    /// Application configs indexed by `audience_hash`
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
                    app.audience_hash
                );
            }
            applications.insert(app.audience_hash, app);
        }

        Ok(Self {
            rpc_url: config.rpc_url,
            chain_id: config.chain_id,
            key_manager_address: config.key_manager_address,
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
        let key_manager_address = cli
            .key_manager_address
            .ok_or_else(|| anyhow::anyhow!("TEE_KEY_MANAGER_CONTRACT_ADDRESS is required"))?;
        let treasury_address = cli
            .treasury_address
            .ok_or_else(|| anyhow::anyhow!("GAS_TREASURY_CONTRACT_ADDRESS is required"))?;
        let private_key = cli
            .private_key
            .ok_or_else(|| anyhow::anyhow!("RELAYER_PRIVATE_KEY is required"))?;
        let audience_hash_str = cli
            .required_audience_hash
            .ok_or_else(|| anyhow::anyhow!("REQUIRED_AUDIENCE_HASH is required"))?;
        let allowed_digests_str = cli
            .allowed_image_digests
            .ok_or_else(|| anyhow::anyhow!("ALLOWED_IMAGE_DIGESTS is required"))?;

        // Parse audience hash
        let audience_hash = parse_b256(&audience_hash_str)
            .ok_or_else(|| anyhow::anyhow!("Invalid REQUIRED_AUDIENCE_HASH format"))?;

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

        let app_config = ApplicationConfig {
            audience_hash,
            treasury_address,
            allowed_image_digests,
            max_funding_per_digest_daily: cli.max_funding_per_digest_daily,
            max_funding_per_address: cli.max_funding_per_address,
        };

        let mut applications = HashMap::new();
        applications.insert(audience_hash, app_config);

        Ok(Self {
            rpc_url,
            chain_id,
            key_manager_address,
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
        assert!(parse_b256("0x123").is_none()); // Too short
        assert!(parse_b256("not_hex").is_none());
    }

    #[test]
    fn test_config_file_parse() {
        // Note: u128 values must be quoted as strings in TOML
        let toml = r#"
            rpc_url = "http://localhost:8545"
            chain_id = 31337
            key_manager_address = "0x9fE46736679d2D9a65F0992F2272dE9f3c7fa6e0"
            private_key = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"

            [[application]]
            audience_hash = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            treasury_address = "0x5FbDB2315678afecb367f032d93F642f64180aa3"
            allowed_image_digests = [
                "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "0xcccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
            ]

            [[application]]
            audience_hash = "0xdddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
            treasury_address = "0xe7f1725E7734CE288F8367e1Bb143E90bb3F0512"
            allowed_image_digests = [
                "0xeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"
            ]
            max_funding_per_digest_daily = "500000000000000000"
            max_funding_per_address = "25000000000000000"
        "#;

        let config: ConfigFile = toml::from_str(toml).unwrap();
        assert_eq!(config.applications.len(), 2);
        assert_eq!(config.applications[0].allowed_image_digests.len(), 2);
        assert_eq!(config.applications[1].allowed_image_digests.len(), 1);
        assert_eq!(
            config.applications[1].max_funding_per_digest_daily,
            500_000_000_000_000_000
        );
    }
}
