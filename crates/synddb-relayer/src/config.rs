//! Relayer configuration

use alloy::primitives::Address;
use clap::Parser;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;

/// Configuration for the gas funding relayer
#[derive(Debug, Clone, Serialize, Deserialize, Parser)]
pub struct RelayerConfig {
    /// RPC URL for transaction submission
    #[arg(long, env = "RPC_URL")]
    pub rpc_url: String,

    /// Chain ID for EIP-712 domain
    #[arg(long, env = "CHAIN_ID")]
    pub chain_id: u64,

    /// `TeeKeyManager` contract address
    #[arg(long, env = "TEE_KEY_MANAGER_CONTRACT_ADDRESS")]
    pub key_manager_address: Address,

    /// `GasTreasury` contract address
    #[arg(long, env = "GAS_TREASURY_CONTRACT_ADDRESS")]
    pub treasury_address: Address,

    /// Relayer's private key (hex-encoded, for paying gas)
    #[arg(long, env = "RELAYER_PRIVATE_KEY")]
    pub private_key: String,

    /// Listen address for HTTP server
    #[arg(long, env = "RELAYER_LISTEN_ADDR", default_value = "0.0.0.0:8082")]
    pub listen_addr: SocketAddr,

    /// Allowed image digests (comma-separated hex hashes)
    /// Only TEEs running these code versions will be funded
    #[arg(long, env = "ALLOWED_IMAGE_DIGESTS")]
    pub allowed_image_digests: String,

    /// Maximum total funding per image digest per day (in wei)
    #[arg(
        long,
        env = "MAX_FUNDING_PER_DIGEST_DAILY",
        default_value = "1000000000000000000"
    )]
    pub max_funding_per_digest_daily: u128, // 1 ETH default

    /// Maximum funding per address (in wei)
    #[arg(
        long,
        env = "MAX_FUNDING_PER_ADDRESS",
        default_value = "50000000000000000"
    )]
    pub max_funding_per_address: u128, // 0.05 ETH default
}

impl RelayerConfig {
    /// Validate the configuration
    pub fn validate(&self) -> anyhow::Result<()> {
        // Validate private key format
        let key = self.private_key.trim_start_matches("0x");
        if key.len() != 64 {
            anyhow::bail!("RELAYER_PRIVATE_KEY must be a 32-byte hex string");
        }
        hex::decode(key)?;

        // Validate at least one allowed digest
        if self.allowed_image_digests.is_empty() {
            anyhow::bail!("ALLOWED_IMAGE_DIGESTS must contain at least one digest");
        }

        // Validate digest format
        for digest in self.allowed_image_digests.split(',') {
            let digest = digest.trim().trim_start_matches("0x");
            if digest.len() != 64 {
                anyhow::bail!("Invalid image digest format: {}", digest);
            }
            hex::decode(digest)?;
        }

        Ok(())
    }

    /// Parse allowed image digests into a set of B256 hashes
    pub fn parse_allowed_digests(&self) -> Vec<alloy::primitives::B256> {
        self.allowed_image_digests
            .split(',')
            .filter_map(|s| {
                let s = s.trim().trim_start_matches("0x");
                hex::decode(s).ok().and_then(|bytes| {
                    (bytes.len() == 32).then(|| alloy::primitives::B256::from_slice(&bytes))
                })
            })
            .collect()
    }
}
