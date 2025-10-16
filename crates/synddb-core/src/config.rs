//! Configuration management for SyndDB
//!
//! This module provides configuration structures for all components of SyndDB,
//! with support for loading from files, environment variables, and defaults.

use crate::types::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// ============================================================================
// Main Configuration
// ============================================================================

/// Root configuration for SyndDB
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyndDBConfig {
    /// Node role (sequencer or replica)
    pub role: NodeRole,
    /// Database configuration
    pub database: DatabaseConfig,
    /// Sequencer-specific configuration
    #[serde(default)]
    pub sequencer: Option<SequencerConfig>,
    /// Replica-specific configuration
    #[serde(default)]
    pub replica: Option<ReplicaConfig>,
    /// Blockchain configuration
    pub chain: ChainConfig,
    /// Storage configuration
    pub storage: StorageConfig,
}

/// Node role
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeRole {
    Sequencer,
    Replica,
}

// ============================================================================
// Database Configuration
// ============================================================================

/// Database configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseConfig {
    /// Path to SQLite database file
    pub path: PathBuf,

    /// Connection pool size
    #[serde(default = "default_pool_size")]
    pub pool_size: u32,

    /// Journal mode (WAL recommended for concurrency)
    #[serde(default = "default_journal_mode")]
    pub journal_mode: String,

    /// Synchronous mode (NORMAL recommended for blockchain use)
    #[serde(default = "default_synchronous")]
    pub synchronous: String,

    /// Cache size in KB (negative for KB, positive for pages)
    #[serde(default = "default_cache_size")]
    pub cache_size: i64,

    /// Memory-mapped I/O size in bytes
    #[serde(default = "default_mmap_size")]
    pub mmap_size: i64,

    /// Locking mode (EXCLUSIVE for production, NORMAL for development)
    #[serde(default = "default_locking_mode")]
    pub locking_mode: String,
}

fn default_pool_size() -> u32 {
    4 // SQLite with WAL: 1 writer + 3 concurrent readers is optimal for sequencer
}

fn default_journal_mode() -> String {
    "WAL".to_string()
}

fn default_synchronous() -> String {
    "NORMAL".to_string()
}

fn default_cache_size() -> i64 {
    -2000000 // 2GB
}

fn default_mmap_size() -> i64 {
    274877906944 // 256GB
}

fn default_locking_mode() -> String {
    "EXCLUSIVE".to_string()
}

// ============================================================================
// Sequencer Configuration
// ============================================================================

/// Sequencer-specific configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SequencerConfig {
    /// Number of operations to batch before publishing
    #[serde(default = "default_batch_size")]
    pub batch_size: usize,

    /// Timeout in milliseconds before publishing a partial batch
    #[serde(default = "default_batch_timeout_ms")]
    pub batch_timeout_ms: u64,

    /// Compression algorithm (zstd)
    #[serde(default = "default_compression")]
    pub compression: String,

    /// Interval in milliseconds between blockchain publications
    #[serde(default = "default_publish_interval_ms")]
    pub publish_interval_ms: u64,

    /// Maximum diff size before creating a snapshot (bytes)
    #[serde(default = "default_max_diff_size")]
    pub max_diff_size: usize,

    /// Blockchain snapshot interval (operations)
    #[serde(default = "default_blockchain_snapshot_interval")]
    pub blockchain_snapshot_interval: u64,
}

fn default_batch_size() -> usize {
    1000
}

fn default_batch_timeout_ms() -> u64 {
    100
}

fn default_compression() -> String {
    "zstd".to_string()
}

fn default_publish_interval_ms() -> u64 {
    1000
}

fn default_max_diff_size() -> usize {
    1048576 // 1MB
}

fn default_blockchain_snapshot_interval() -> u64 {
    10000
}

impl Default for SequencerConfig {
    fn default() -> Self {
        Self {
            batch_size: default_batch_size(),
            batch_timeout_ms: default_batch_timeout_ms(),
            compression: default_compression(),
            publish_interval_ms: default_publish_interval_ms(),
            max_diff_size: default_max_diff_size(),
            blockchain_snapshot_interval: default_blockchain_snapshot_interval(),
        }
    }
}

// ============================================================================
// Replica Configuration
// ============================================================================

/// Replica-specific configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplicaConfig {
    /// Sync interval in milliseconds
    #[serde(default = "default_sync_interval_ms")]
    pub sync_interval_ms: u64,

    /// Cache TTL in milliseconds
    #[serde(default = "default_cache_ttl_ms")]
    pub cache_ttl_ms: u64,

    /// Maximum lag in blocks before alerting
    #[serde(default = "default_max_lag_blocks")]
    pub max_lag_blocks: u64,

    /// Query server port
    #[serde(default = "default_query_port")]
    pub query_port: u16,
}

fn default_sync_interval_ms() -> u64 {
    500
}

fn default_cache_ttl_ms() -> u64 {
    60000
}

fn default_max_lag_blocks() -> u64 {
    100
}

fn default_query_port() -> u16 {
    3000
}

impl Default for ReplicaConfig {
    fn default() -> Self {
        Self {
            sync_interval_ms: default_sync_interval_ms(),
            cache_ttl_ms: default_cache_ttl_ms(),
            max_lag_blocks: default_max_lag_blocks(),
            query_port: default_query_port(),
        }
    }
}

// ============================================================================
// Blockchain Configuration
// ============================================================================

/// Blockchain configuration
///
/// This config integrates with Alloy's wallet provider, which handles:
/// - Automatic gas estimation via `eth_estimateGas`
/// - EIP-1559 fee estimation from the network
/// - Transaction signing and submission
///
/// We trust Alloy's gas estimation and only configure:
/// - Fee caps (protection against market conditions)
/// - Confirmation requirements (security vs latency tradeoff)
/// - Polling behavior (how often to check transaction status)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainConfig {
    /// RPC URL for the blockchain
    pub rpc_url: String,

    /// Smart contract address
    pub contract_address: String,

    /// Private key for signing transactions (use env var in production)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub private_key: Option<String>,

    /// Maximum priority fee per gas in gwei (EIP-1559)
    /// Optional cap to protect against MEV or market volatility
    /// If None, uses network-suggested values
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_priority_fee_per_gas: Option<u64>,

    /// Maximum fee per gas in gwei (EIP-1559)
    /// Optional cap to protect against gas price spikes
    /// If None, uses network-suggested values
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_fee_per_gas: Option<u64>,

    /// Confirmation blocks to wait before considering transaction final
    ///
    /// Security vs Latency tradeoff for L2 deployments (e.g., Syndicate Chain):
    /// - 1 block: Fast (~250ms on Syndicate Chain) but vulnerable to single-block re-orgs
    /// - 3-10 blocks: Balanced security for most production applications (~750ms-2.5s)
    /// - ~345600 blocks: 24-hour Arbitrum Orbit batch posting window at 250ms/block (very safe)
    /// - L1 finality: Wait for L2→L1 state root + L1 finalization (safest, hours/days)
    ///
    /// Note: Syndicate Chain and many Arbitrum Orbit chains run at 250ms block times.
    /// For L1 deployments: 1=fast, 6=reasonable, 32+=finality (2 epochs)
    #[serde(default = "default_confirmations")]
    pub confirmations: u64,

    /// Transaction polling interval in milliseconds
    #[serde(default = "default_poll_interval_ms")]
    pub poll_interval_ms: u64,
}

fn default_confirmations() -> u64 {
    1 // Wait for 1 block confirmation
}

fn default_poll_interval_ms() -> u64 {
    1000 // Poll every second
}

// ============================================================================
// Storage Configuration
// ============================================================================

/// Storage provider configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    /// Storage provider (ipfs or arweave)
    #[serde(default = "default_storage_provider")]
    pub provider: String,

    /// IPFS configuration
    #[serde(default)]
    pub ipfs: Option<IPFSConfig>,

    /// Arweave configuration
    #[serde(default)]
    pub arweave: Option<ArweaveConfig>,
}

fn default_storage_provider() -> String {
    "ipfs".to_string()
}

/// IPFS storage configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IPFSConfig {
    /// IPFS gateway URL
    #[serde(default = "default_ipfs_gateway")]
    pub gateway: String,

    /// IPFS API endpoint
    #[serde(default = "default_ipfs_api")]
    pub api_endpoint: String,
}

fn default_ipfs_gateway() -> String {
    "https://ipfs.io".to_string()
}

fn default_ipfs_api() -> String {
    "http://localhost:5001".to_string()
}

impl Default for IPFSConfig {
    fn default() -> Self {
        Self {
            gateway: default_ipfs_gateway(),
            api_endpoint: default_ipfs_api(),
        }
    }
}

/// Arweave storage configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArweaveConfig {
    /// Arweave gateway URL
    #[serde(default = "default_arweave_gateway")]
    pub gateway: String,

    /// Path to Arweave wallet file
    pub wallet_path: Option<PathBuf>,
}

fn default_arweave_gateway() -> String {
    "https://arweave.net".to_string()
}

impl Default for ArweaveConfig {
    fn default() -> Self {
        Self {
            gateway: default_arweave_gateway(),
            wallet_path: None,
        }
    }
}

// ============================================================================
// Configuration Loading
// ============================================================================

impl SyndDBConfig {
    /// Load configuration from a file
    ///
    /// This loads configuration from multiple sources in priority order:
    /// 1. Environment variables with SYNDDB_ prefix (highest priority)
    /// 2. Config file (YAML/TOML/JSON)
    /// 3. Defaults defined in code
    ///
    /// In development, a .env file will be automatically loaded if present.
    /// The .env file should never be committed to version control.
    pub fn from_file(path: &str) -> Result<Self> {
        // Load .env file if it exists (development only - fails silently in production)
        dotenvy::dotenv().ok();

        let config = config::Config::builder()
            .add_source(config::File::with_name(path))
            .add_source(config::Environment::with_prefix("SYNDDB"))
            .build()?;

        let synddb_config: SyndDBConfig = config.try_deserialize()?;
        Ok(synddb_config)
    }

    /// Create a default configuration for testing
    pub fn test_config() -> Self {
        Self {
            role: NodeRole::Sequencer,
            database: DatabaseConfig {
                path: PathBuf::from("test.db"),
                pool_size: 4,
                journal_mode: "WAL".to_string(),
                synchronous: "NORMAL".to_string(),
                cache_size: -64000,       // 64MB for tests
                mmap_size: 1073741824,    // 1GB for tests
                locking_mode: "NORMAL".to_string(), // Allow external access in tests
            },
            sequencer: Some(SequencerConfig::default()),
            replica: None,
            chain: ChainConfig {
                rpc_url: "http://localhost:8545".to_string(),
                contract_address: "0x0000000000000000000000000000000000000000".to_string(),
                private_key: Some(
                    "0x0000000000000000000000000000000000000000000000000000000000000001"
                        .to_string(),
                ),
                max_priority_fee_per_gas: None, // Use Alloy's estimates
                max_fee_per_gas: None,          // Use Alloy's estimates
                confirmations: 1,
                poll_interval_ms: 100, // Faster polling for tests
            },
            storage: StorageConfig {
                provider: "ipfs".to_string(),
                ipfs: Some(IPFSConfig::default()),
                arweave: None,
            },
        }
    }

    /// Validate the configuration
    pub fn validate(&self) -> Result<()> {
        // Validate role-specific configuration
        match self.role {
            NodeRole::Sequencer => {
                if self.sequencer.is_none() {
                    return Err(crate::types::Error::Config(config::ConfigError::Message(
                        "Sequencer role requires sequencer configuration".to_string(),
                    )));
                }
            }
            NodeRole::Replica => {
                if self.replica.is_none() {
                    return Err(crate::types::Error::Config(config::ConfigError::Message(
                        "Replica role requires replica configuration".to_string(),
                    )));
                }
            }
        }

        // Validate storage provider
        match self.storage.provider.as_str() {
            "ipfs" => {
                if self.storage.ipfs.is_none() {
                    return Err(crate::types::Error::Config(config::ConfigError::Message(
                        "IPFS provider requires ipfs configuration".to_string(),
                    )));
                }
            }
            "arweave" => {
                if self.storage.arweave.is_none() {
                    return Err(crate::types::Error::Config(config::ConfigError::Message(
                        "Arweave provider requires arweave configuration".to_string(),
                    )));
                }
            }
            _ => {
                return Err(crate::types::Error::Config(config::ConfigError::Message(
                    format!("Unknown storage provider: {}", self.storage.provider),
                )));
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_configs() {
        let sequencer_config = SequencerConfig::default();
        assert_eq!(sequencer_config.batch_size, 1000);

        let replica_config = ReplicaConfig::default();
        assert_eq!(replica_config.sync_interval_ms, 500);
    }

    #[test]
    fn test_test_config() {
        let config = SyndDBConfig::test_config();
        assert_eq!(config.role, NodeRole::Sequencer);
        assert!(config.sequencer.is_some());
    }

    #[test]
    fn test_config_validation() {
        let mut config = SyndDBConfig::test_config();
        assert!(config.validate().is_ok());

        // Test invalid configuration
        config.sequencer = None;
        assert!(config.validate().is_err());
    }
}
