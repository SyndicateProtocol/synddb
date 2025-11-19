//! Configuration structures for the sequencer

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub database: DatabaseConfig,
    pub batch: BatchConfig,
    pub publish: PublishConfig,
    pub messages: MessageConfig,
    pub tee: TeeConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseConfig {
    /// Path to the SQLite database to monitor
    pub path: PathBuf,
    /// Whether to enable Session Extension monitoring
    pub enable_sessions: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchConfig {
    /// Maximum batch size in bytes (default: 1MB)
    pub max_batch_size: usize,

    /// Maximum batch age before flushing (default: 1 second)
    #[serde(with = "humantime_serde")]
    pub max_batch_age: Duration,

    /// Snapshot interval (default: 60 minutes)
    #[serde(with = "humantime_serde")]
    pub snapshot_interval: Duration,

    /// Number of batches before forcing snapshot (default: 1000)
    pub snapshot_threshold: usize,
}

impl Default for BatchConfig {
    fn default() -> Self {
        BatchConfig {
            max_batch_size: default_max_batch_size(),
            max_batch_age: default_max_batch_age(),
            snapshot_interval: default_snapshot_interval(),
            snapshot_threshold: default_snapshot_threshold(),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishConfig {
    /// Enable Celestia DA layer
    pub celestia: Option<CelestiaConfig>,

    /// Enable EigenDA layer
    pub eigenda: Option<EigenDAConfig>,

    /// Enable IPFS storage
    pub ipfs: Option<IpfsConfig>,

    /// Enable Arweave storage
    pub arweave: Option<ArweaveConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CelestiaConfig {
    pub rpc_url: String,
    pub namespace: String,
    pub auth_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EigenDAConfig {
    pub rpc_url: String,
    pub disperser_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpfsConfig {
    pub api_url: String,
    pub gateway_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArweaveConfig {
    pub gateway_url: String,
    pub wallet_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageConfig {
    /// Enable inbound message monitoring from blockchain
    pub enable_inbound: bool,

    /// Enable outbound message monitoring from SQLite tables
    pub enable_outbound: bool,

    /// HTTP API port for message delivery to application
    #[serde(default = "default_api_port")]
    pub api_port: u16,

    /// Chain RPC URL for monitoring deposits
    pub chain_rpc_url: Option<String>,

    /// Bridge contract address
    pub bridge_contract: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeeConfig {
    /// Enable TEE attestation
    pub enable_attestation: bool,

    /// GCP project ID (for Confidential Space)
    pub gcp_project_id: Option<String>,

    /// Path to signing key (managed in TEE)
    pub key_path: Option<PathBuf>,
}

// Default value functions
fn default_max_batch_size() -> usize {
    1024 * 1024 // 1MB
}

fn default_max_batch_age() -> Duration {
    Duration::from_secs(1)
}

fn default_snapshot_interval() -> Duration {
    Duration::from_secs(60 * 60) // 60 minutes
}

fn default_snapshot_threshold() -> usize {
    1000
}

fn default_api_port() -> u16 {
    8432
}

impl Default for Config {
    fn default() -> Self {
        Self {
            database: DatabaseConfig {
                path: PathBuf::from("app.db"),
                enable_sessions: true,
            },
            batch: BatchConfig {
                max_batch_size: default_max_batch_size(),
                max_batch_age: default_max_batch_age(),
                snapshot_interval: default_snapshot_interval(),
                snapshot_threshold: default_snapshot_threshold(),
            },
            publish: PublishConfig {
                celestia: None,
                eigenda: None,
                ipfs: None,
                arweave: None,
            },
            messages: MessageConfig {
                enable_inbound: false,
                enable_outbound: false,
                api_port: default_api_port(),
                chain_rpc_url: None,
                bridge_contract: None,
            },
            tee: TeeConfig {
                enable_attestation: false,
                gcp_project_id: None,
                key_path: None,
            },
        }
    }
}

impl Config {
    /// Load configuration from a YAML file
    pub fn from_file(path: &Path) -> Result<Self> {
        let contents = std::fs::read_to_string(path)
            .context(format!("Failed to read config file: {:?}", path))?;
        let config: Config =
            serde_yaml::from_str(&contents).context("Failed to parse config file")?;
        Ok(config)
    }
}
