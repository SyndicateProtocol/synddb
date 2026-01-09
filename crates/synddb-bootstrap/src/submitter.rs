//! Contract interaction for key verification and draining

use crate::{BootstrapConfig, BootstrapError};
use alloy::{
    network::EthereumWallet,
    primitives::{Address, B256},
    providers::{Provider, ProviderBuilder},
    signers::local::PrivateKeySigner,
    sol,
};
use std::str::FromStr;
use tracing::{debug, info, warn};

// Bridge contract interface for key validation
sol! {
    #[sol(rpc)]
    interface IBridge {
        function teeKeyManager() external view returns (address);
    }

    #[sol(rpc)]
    interface ITeeKeyManager {
        /// KeyType enum: 0 = Sequencer, 1 = Validator
        function isKeyValid(uint8 keyType, address publicKey) external view returns (bool);
    }
}

/// Check if an error string indicates an `InvalidPublicKey` error.
///
/// The `TeeKeyManager` contract reverts with `InvalidPublicKey(address)` when a key
/// is not registered. This error can appear in two forms in error messages:
/// - The decoded name: `InvalidPublicKey`
/// - The hex selector: `0xffc44e88` (`keccak256("InvalidPublicKey(address)")[:4]`)
fn is_invalid_public_key_error(err_str: &str) -> bool {
    err_str.contains("InvalidPublicKey") || err_str.contains("0xffc44e88")
}

/// Contract submitter for key verification and draining
#[derive(Debug)]
pub struct ContractSubmitter {
    rpc_url: String,
    bridge_address: Address,
}

impl ContractSubmitter {
    /// Create a new contract submitter from config
    pub fn from_config(config: &BootstrapConfig) -> Result<Self, BootstrapError> {
        let rpc_url = config
            .rpc_url
            .clone()
            .ok_or_else(|| BootstrapError::Config("BOOTSTRAP_RPC_URL is required".into()))?;

        let bridge_address: Address = config
            .bridge_address
            .as_ref()
            .ok_or_else(|| BootstrapError::Config("BRIDGE_CONTRACT_ADDRESS is required".into()))?
            .parse()
            .map_err(|e| BootstrapError::Config(format!("Invalid bridge address: {e}")))?;

        Ok(Self {
            rpc_url,
            bridge_address,
        })
    }

    /// Get the balance of an address
    pub async fn get_balance(&self, address: Address) -> Result<u128, BootstrapError> {
        let url = reqwest::Url::from_str(&self.rpc_url)
            .map_err(|e| BootstrapError::Config(format!("Invalid RPC URL: {e}")))?;

        let provider = ProviderBuilder::new().connect_http(url);

        let balance = provider
            .get_balance(address)
            .await
            .map_err(|e| BootstrapError::ContractSubmissionFailed(e.to_string()))?;

        Ok(balance.to::<u128>())
    }

    /// Verify that a sequencer key is registered on-chain
    pub async fn is_sequencer_key_valid(&self, address: Address) -> Result<bool, BootstrapError> {
        let url = reqwest::Url::from_str(&self.rpc_url)
            .map_err(|e| BootstrapError::Config(format!("Invalid RPC URL: {e}")))?;

        let provider = ProviderBuilder::new().connect_http(url);

        let tee_key_manager_address = self.fetch_tee_key_manager_address(&provider).await?;
        let contract = ITeeKeyManager::new(tee_key_manager_address, &provider);

        // KeyType::Sequencer = 0
        match contract.isKeyValid(0, address).call().await {
            Ok(_) => Ok(true),
            Err(e) => {
                if is_invalid_public_key_error(&e.to_string()) {
                    Ok(false)
                } else {
                    Err(BootstrapError::KeyVerificationFailed(e.to_string()))
                }
            }
        }
    }

    /// Verify that a validator key is registered on-chain
    pub async fn is_validator_key_valid(&self, address: Address) -> Result<bool, BootstrapError> {
        let url = reqwest::Url::from_str(&self.rpc_url)
            .map_err(|e| BootstrapError::Config(format!("Invalid RPC URL: {e}")))?;

        let provider = ProviderBuilder::new().connect_http(url);

        let tee_key_manager_address = self.fetch_tee_key_manager_address(&provider).await?;
        let contract = ITeeKeyManager::new(tee_key_manager_address, &provider);

        // KeyType::Validator = 1
        match contract.isKeyValid(1, address).call().await {
            Ok(_) => Ok(true),
            Err(e) => {
                if is_invalid_public_key_error(&e.to_string()) {
                    Ok(false)
                } else {
                    Err(BootstrapError::KeyVerificationFailed(e.to_string()))
                }
            }
        }
    }

    /// Fetch `TeeKeyManager` address from Bridge contract
    async fn fetch_tee_key_manager_address<P: Provider>(
        &self,
        provider: &P,
    ) -> Result<Address, BootstrapError> {
        let bridge = IBridge::new(self.bridge_address, provider);
        bridge
            .teeKeyManager()
            .call()
            .await
            .map(|r| Address::from(r.0))
            .map_err(|e| {
                BootstrapError::KeyVerificationFailed(format!(
                    "Failed to fetch TeeKeyManager address: {e}"
                ))
            })
    }

    /// Get current gas price
    pub async fn get_gas_price(&self) -> Result<u128, BootstrapError> {
        let url = reqwest::Url::from_str(&self.rpc_url)
            .map_err(|e| BootstrapError::Config(format!("Invalid RPC URL: {e}")))?;

        let provider = ProviderBuilder::new().connect_http(url);

        let gas_price = provider
            .get_gas_price()
            .await
            .map_err(|e| BootstrapError::ContractSubmissionFailed(e.to_string()))?;

        Ok(gas_price)
    }

    /// Send ETH from one address to another
    ///
    /// Used for draining old keys to treasury.
    pub async fn send_eth(
        &self,
        signer: &PrivateKeySigner,
        to: Address,
        amount: u128,
    ) -> Result<B256, BootstrapError> {
        use alloy::rpc::types::TransactionRequest;

        let wallet = EthereumWallet::from(signer.clone());
        let url = reqwest::Url::from_str(&self.rpc_url)
            .map_err(|e| BootstrapError::Config(format!("Invalid RPC URL: {e}")))?;

        let provider = ProviderBuilder::new().wallet(wallet).connect_http(url);

        let tx = TransactionRequest::default()
            .to(to)
            .value(alloy::primitives::U256::from(amount));

        let pending = provider
            .send_transaction(tx)
            .await
            .map_err(|e| BootstrapError::TransactionFailed(e.to_string()))?;

        let tx_hash = *pending.tx_hash();
        info!(
            tx_hash = %tx_hash,
            to = %to,
            amount_wei = amount,
            "ETH transfer submitted"
        );

        Ok(tx_hash)
    }

    /// Wait for transaction confirmation
    pub async fn wait_for_confirmation(
        &self,
        tx_hash: B256,
        timeout: std::time::Duration,
    ) -> Result<(), BootstrapError> {
        let url = reqwest::Url::from_str(&self.rpc_url)
            .map_err(|e| BootstrapError::Config(format!("Invalid RPC URL: {e}")))?;

        let provider = ProviderBuilder::new().connect_http(url);

        let start = std::time::Instant::now();
        let poll_interval = std::time::Duration::from_secs(2);

        loop {
            if start.elapsed() > timeout {
                return Err(BootstrapError::TransactionConfirmationFailed(
                    "Timeout waiting for confirmation".into(),
                ));
            }

            match provider.get_transaction_receipt(tx_hash).await {
                Ok(Some(receipt)) => {
                    if receipt.status() {
                        info!(tx_hash = %tx_hash, "Transaction confirmed successfully");
                        return Ok(());
                    }
                    return Err(BootstrapError::TransactionFailed(
                        "Transaction reverted".into(),
                    ));
                }
                Ok(None) => {
                    debug!(tx_hash = %tx_hash, "Transaction pending...");
                }
                Err(e) => {
                    warn!(error = %e, "Error checking transaction receipt");
                }
            }

            tokio::time::sleep(poll_interval).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_validation() {
        let config = BootstrapConfig {
            enable_key_bootstrap: true,
            bridge_address: Some("0x1234567890123456789012345678901234567890".into()),
            rpc_url: Some("http://localhost:8545".into()),
            chain_id: Some(1),
            relayer_url: Some("http://localhost:8082".into()),
            ..Default::default()
        };

        let submitter = ContractSubmitter::from_config(&config);
        assert!(submitter.is_ok());
    }

    #[test]
    fn test_missing_rpc_url() {
        let config = BootstrapConfig {
            enable_key_bootstrap: true,
            bridge_address: Some("0x1234567890123456789012345678901234567890".into()),
            ..Default::default()
        };

        let submitter = ContractSubmitter::from_config(&config);
        assert!(submitter.is_err());
    }
}
