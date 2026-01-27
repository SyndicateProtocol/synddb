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

    /// Bridge contract address (for key registration)
    #[arg(long, env = "BRIDGE_CONTRACT_ADDRESS")]
    pub bridge_address: Option<String>,

    /// RPC endpoint for verifying key registration
    #[arg(long, env = "BOOTSTRAP_RPC_URL")]
    pub rpc_url: Option<String>,

    /// Chain ID for EIP-712 signatures
    #[arg(long, env = "BOOTSTRAP_CHAIN_ID")]
    pub chain_id: Option<u64>,

    /// Relayer URL for key registration (relayer pays gas)
    #[arg(long, env = "RELAYER_URL")]
    pub relayer_url: Option<String>,

    /// Proof generation mode
    #[arg(long, env = "PROVER_MODE", default_value = "service", value_enum)]
    pub prover_mode: ProverMode,

    /// Proof service URL (required when `prover_mode=service`)
    #[arg(long, env = "PROOF_SERVICE_URL")]
    pub proof_service_url: Option<String>,

    /// Attestation audience (defaults to service URL)
    #[arg(long, env = "ATTESTATION_AUDIENCE")]
    pub attestation_audience: Option<String>,

    /// Timeout for proof generation (network proofs can take 10-20 minutes)
    #[arg(long, env = "PROOF_TIMEOUT", default_value = "20m", value_parser = humantime::parse_duration)]
    #[serde(with = "humantime_serde")]
    pub proof_timeout: Duration,

    /// Timeout for proof service health checks (default: 10 seconds)
    #[arg(long, env = "PROOF_HEALTH_CHECK_TIMEOUT", default_value = "10s", value_parser = humantime::parse_duration)]
    #[serde(with = "humantime_serde")]
    pub proof_health_check_timeout: Duration,

    /// Total bootstrap timeout (must exceed `proof_timeout` + registration time)
    #[arg(long, env = "BOOTSTRAP_TIMEOUT", default_value = "30m", value_parser = humantime::parse_duration)]
    #[serde(with = "humantime_serde")]
    pub bootstrap_timeout: Duration,

    /// Maximum retries for proof generation
    #[arg(long, env = "PROOF_MAX_RETRIES", default_value = "3")]
    pub proof_max_retries: u32,

    /// Maximum retries for relayer requests
    #[arg(long, env = "RELAYER_MAX_RETRIES", default_value = "3")]
    pub relayer_max_retries: u32,

    /// Maximum retries for key verification after registration
    ///
    /// RPC nodes may have a delay before reflecting newly registered keys.
    /// This retry loop accounts for that indexing delay.
    #[arg(long, env = "VERIFICATION_MAX_RETRIES", default_value = "5")]
    pub verification_max_retries: u32,

    /// Timeout for relayer key registration requests (default: 3 minutes)
    ///
    /// This covers the time for the relayer to submit the transaction and
    /// wait for on-chain confirmation.
    #[arg(long, env = "RELAYER_TIMEOUT", default_value = "3m", value_parser = humantime::parse_duration)]
    #[serde(with = "humantime_serde")]
    pub relayer_timeout: Duration,

    /// Image signature over the image digest hash (65 bytes r||s||v, hex-encoded with 0x prefix)
    /// This is the secp256k1 ECDSA signature produced by signing the keccak256 hash of the
    /// image digest string (e.g., "sha256:abc123...") using an Ethereum key.
    #[arg(long, env = "IMAGE_SIGNATURE")]
    pub image_signature: Option<String>,
}

impl Default for BootstrapConfig {
    fn default() -> Self {
        Self {
            enable_key_bootstrap: false,
            bridge_address: None,
            rpc_url: None,
            chain_id: None,
            relayer_url: None,
            prover_mode: ProverMode::Service,
            proof_service_url: None,
            attestation_audience: None,
            proof_timeout: Duration::from_secs(1200),
            proof_health_check_timeout: Duration::from_secs(10),
            bootstrap_timeout: Duration::from_secs(1800),
            proof_max_retries: 3,
            relayer_max_retries: 3,
            verification_max_retries: 5,
            relayer_timeout: Duration::from_secs(180),
            image_signature: None,
        }
    }
}

impl BootstrapConfig {
    /// Validate configuration for production use
    pub fn validate(&self) -> Result<(), crate::BootstrapError> {
        if !self.enable_key_bootstrap {
            return Ok(());
        }

        if self.bridge_address.is_none() {
            return Err(crate::BootstrapError::Config(
                "BRIDGE_CONTRACT_ADDRESS is required when bootstrap is enabled".into(),
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

        if self.relayer_url.is_none() {
            return Err(crate::BootstrapError::Config(
                "RELAYER_URL is required when bootstrap is enabled".into(),
            ));
        }

        if self.prover_mode == ProverMode::Service && self.proof_service_url.is_none() {
            return Err(crate::BootstrapError::Config(
                "PROOF_SERVICE_URL is required when prover_mode is 'service'".into(),
            ));
        }

        // Image signature is required for on-chain verification
        if self.image_signature.is_none() {
            return Err(crate::BootstrapError::Config(
                "IMAGE_SIGNATURE is required when bootstrap is enabled".into(),
            ));
        }

        Ok(())
    }
}

/// Proof generation mode
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum ProverMode {
    /// Use self-hosted proof service
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
        assert_eq!(config.proof_timeout, Duration::from_secs(1200));
    }

    #[test]
    fn test_validate_disabled() {
        let config = BootstrapConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_missing_bridge() {
        let config = BootstrapConfig {
            enable_key_bootstrap: true,
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_missing_relayer() {
        let config = BootstrapConfig {
            enable_key_bootstrap: true,
            bridge_address: Some("0x1234567890123456789012345678901234567890".into()),
            rpc_url: Some("http://localhost:8545".into()),
            chain_id: Some(1),
            proof_service_url: Some("http://localhost:8080".into()),
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }
}
