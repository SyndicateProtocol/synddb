//! Bootstrap configuration

use clap::{Parser, ValueEnum};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Configuration for key bootstrapping
#[derive(Debug, Clone, Serialize, Deserialize, Parser)]
pub struct BootstrapConfig {
    /// Enable key bootstrapping (required in production TEE)
    #[arg(long, env = "ENABLE_KEY_BOOTSTRAP", default_value = "false")]
    pub enable_key_bootstrap: bool,

    /// `TeeKeyManager` contract address
    #[arg(long, env = "TEE_KEY_MANAGER_ADDRESS")]
    pub tee_key_manager_address: Option<String>,

    /// RPC endpoint for submitting transactions
    #[arg(long, env = "BOOTSTRAP_RPC_URL")]
    pub rpc_url: Option<String>,

    /// Chain ID for contract interactions
    #[arg(long, env = "BOOTSTRAP_CHAIN_ID")]
    pub chain_id: Option<u64>,

    /// Proof generation mode
    #[arg(long, env = "SP1_PROVER_MODE", default_value = "service", value_enum)]
    pub prover_mode: ProverMode,

    /// Proof service URL (required when `prover_mode=service`)
    #[arg(long, env = "PROOF_SERVICE_URL")]
    pub proof_service_url: Option<String>,

    /// Attestation audience (defaults to service URL)
    #[arg(long, env = "ATTESTATION_AUDIENCE")]
    pub attestation_audience: Option<String>,

    /// Timeout for proof generation
    #[arg(long, env = "PROOF_TIMEOUT", default_value = "10m", value_parser = humantime::parse_duration)]
    #[serde(with = "humantime_serde")]
    pub proof_timeout: Duration,

    /// Total bootstrap timeout
    #[arg(long, env = "BOOTSTRAP_TIMEOUT", default_value = "15m", value_parser = humantime::parse_duration)]
    #[serde(with = "humantime_serde")]
    pub bootstrap_timeout: Duration,

    /// Maximum retries for proof generation
    #[arg(long, env = "PROOF_MAX_RETRIES", default_value = "3")]
    pub proof_max_retries: u32,

    /// Maximum retries for transaction submission
    #[arg(long, env = "TX_MAX_RETRIES", default_value = "5")]
    pub tx_max_retries: u32,

    /// Minimum balance required for gas (in wei)
    #[arg(long, env = "MIN_GAS_BALANCE", default_value = "10000000000000000")]
    pub min_gas_balance: u128,
}

impl Default for BootstrapConfig {
    fn default() -> Self {
        Self {
            enable_key_bootstrap: false,
            tee_key_manager_address: None,
            rpc_url: None,
            chain_id: None,
            prover_mode: ProverMode::Service,
            proof_service_url: None,
            attestation_audience: None,
            proof_timeout: Duration::from_secs(600),
            bootstrap_timeout: Duration::from_secs(900),
            proof_max_retries: 3,
            tx_max_retries: 5,
            min_gas_balance: 10_000_000_000_000_000, // 0.01 ETH
        }
    }
}

impl BootstrapConfig {
    /// Validate configuration for production use
    pub fn validate(&self) -> Result<(), crate::BootstrapError> {
        if !self.enable_key_bootstrap {
            return Ok(());
        }

        if self.tee_key_manager_address.is_none() {
            return Err(crate::BootstrapError::Config(
                "TEE_KEY_MANAGER_ADDRESS is required when bootstrap is enabled".into(),
            ));
        }

        if self.rpc_url.is_none() {
            return Err(crate::BootstrapError::Config(
                "BOOTSTRAP_RPC_URL is required when bootstrap is enabled".into(),
            ));
        }

        if self.chain_id.is_none() {
            return Err(crate::BootstrapError::Config(
                "BOOTSTRAP_CHAIN_ID is required when bootstrap is enabled".into(),
            ));
        }

        if self.prover_mode == ProverMode::Service && self.proof_service_url.is_none() {
            return Err(crate::BootstrapError::Config(
                "PROOF_SERVICE_URL is required when prover_mode is 'service'".into(),
            ));
        }

        Ok(())
    }
}

/// Proof generation mode
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum ProverMode {
    /// Use self-hosted GPU proof service
    #[default]
    Service,
    /// Use mock prover for testing (no real proof)
    Mock,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = BootstrapConfig::default();
        assert!(!config.enable_key_bootstrap);
        assert_eq!(config.proof_timeout, Duration::from_secs(600));
    }

    #[test]
    fn test_validate_disabled() {
        let config = BootstrapConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_missing_contract() {
        let config = BootstrapConfig {
            enable_key_bootstrap: true,
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }
}
