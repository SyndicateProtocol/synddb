//! Contract submission for key registration

use crate::{BootstrapConfig, BootstrapError, ProofResponse};
use alloy::{
    network::EthereumWallet,
    primitives::{Address, B256},
    providers::{Provider, ProviderBuilder},
    signers::local::PrivateKeySigner,
    sol,
};
use std::{str::FromStr, time::Duration};
use tracing::{debug, info, warn};

// Contract interface for TeeKeyManager
sol! {
    #[sol(rpc)]
    interface ITeeKeyManager {
        function addKey(bytes calldata publicValues, bytes calldata proofBytes) external;
        function isKeyValid(address publicKey) external view returns (bool);
    }
}

/// Submits proofs to the `TeeKeyManager` contract
#[derive(Debug)]
pub struct ContractSubmitter {
    rpc_url: String,
    key_manager_address: Address,
    chain_id: u64,
    min_balance: u128,
}

impl ContractSubmitter {
    /// Create a new contract submitter from config
    pub fn from_config(config: &BootstrapConfig) -> Result<Self, BootstrapError> {
        let rpc_url = config
            .rpc_url
            .clone()
            .ok_or_else(|| BootstrapError::Config("BOOTSTRAP_RPC_URL is required".into()))?;

        let key_manager_address: Address = config
            .tee_key_manager_address
            .as_ref()
            .ok_or_else(|| {
                BootstrapError::Config("TEE_KEY_MANAGER_CONTRACT_ADDRESS is required".into())
            })?
            .parse()
            .map_err(|e| BootstrapError::Config(format!("Invalid contract address: {e}")))?;

        let chain_id = config
            .chain_id
            .ok_or_else(|| BootstrapError::Config("BOOTSTRAP_CHAIN_ID is required".into()))?;

        Ok(Self {
            rpc_url,
            key_manager_address,
            chain_id,
            min_balance: config.min_gas_balance,
        })
    }

    /// Check the balance of an address
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

    /// Check if balance is sufficient for gas
    pub async fn check_balance(&self, address: Address) -> Result<(), BootstrapError> {
        let balance = self.get_balance(address).await?;

        if balance < self.min_balance {
            return Err(BootstrapError::InsufficientBalance {
                have: balance,
                need: self.min_balance,
            });
        }

        debug!(
            address = %address,
            balance_wei = balance,
            min_balance_wei = self.min_balance,
            "Balance check passed"
        );

        Ok(())
    }

    /// Submit key registration to the contract
    ///
    /// The TEE key signs and pays for the transaction.
    pub async fn submit_key_registration(
        &self,
        proof: &ProofResponse,
        signer: &PrivateKeySigner,
    ) -> Result<B256, BootstrapError> {
        let wallet = EthereumWallet::from(signer.clone());
        let url = reqwest::Url::from_str(&self.rpc_url)
            .map_err(|e| BootstrapError::Config(format!("Invalid RPC URL: {e}")))?;

        let provider = ProviderBuilder::new().wallet(wallet).connect_http(url);

        // Parse hex-encoded proof data
        let public_values =
            hex::decode(proof.public_values.trim_start_matches("0x")).map_err(|e| {
                BootstrapError::ProofGenerationFailed(format!("Invalid public_values hex: {e}"))
            })?;

        let proof_bytes = hex::decode(proof.proof_bytes.trim_start_matches("0x")).map_err(|e| {
            BootstrapError::ProofGenerationFailed(format!("Invalid proof_bytes hex: {e}"))
        })?;

        info!(
            contract = %self.key_manager_address,
            public_values_len = public_values.len(),
            proof_bytes_len = proof_bytes.len(),
            "Submitting addKey transaction"
        );

        let contract = ITeeKeyManager::new(self.key_manager_address, &provider);

        let tx = contract.addKey(public_values.into(), proof_bytes.into());

        let pending = tx.send().await.map_err(|e: alloy::contract::Error| {
            BootstrapError::TransactionFailed(e.to_string())
        })?;

        let tx_hash = *pending.tx_hash();
        info!(tx_hash = %tx_hash, "Transaction submitted, awaiting confirmation");

        Ok(tx_hash)
    }

    /// Wait for transaction confirmation
    pub async fn wait_for_confirmation(
        &self,
        tx_hash: B256,
        timeout: Duration,
    ) -> Result<(), BootstrapError> {
        let url = reqwest::Url::from_str(&self.rpc_url)
            .map_err(|e| BootstrapError::Config(format!("Invalid RPC URL: {e}")))?;

        let provider = ProviderBuilder::new().connect_http(url);

        let start = std::time::Instant::now();
        let poll_interval = Duration::from_secs(2);

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

    /// Verify that a key is registered on-chain
    pub async fn is_key_valid(&self, address: Address) -> Result<bool, BootstrapError> {
        let url = reqwest::Url::from_str(&self.rpc_url)
            .map_err(|e| BootstrapError::Config(format!("Invalid RPC URL: {e}")))?;

        let provider = ProviderBuilder::new().connect_http(url);

        let contract = ITeeKeyManager::new(self.key_manager_address, &provider);

        match contract.isKeyValid(address).call().await {
            Ok(_) => Ok(true),
            Err(e) => {
                let err_str = e.to_string();
                if err_str.contains("InvalidPublicKey") {
                    Ok(false)
                } else {
                    Err(BootstrapError::KeyVerificationFailed(err_str))
                }
            }
        }
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_validation() {
        let config = BootstrapConfig {
            enable_key_bootstrap: true,
            tee_key_manager_address: Some("0x1234567890123456789012345678901234567890".into()),
            rpc_url: Some("http://localhost:8545".into()),
            chain_id: Some(1),
            ..Default::default()
        };

        let submitter = ContractSubmitter::from_config(&config);
        assert!(submitter.is_ok());
    }

    #[test]
    fn test_missing_rpc_url() {
        let config = BootstrapConfig {
            enable_key_bootstrap: true,
            tee_key_manager_address: Some("0x1234567890123456789012345678901234567890".into()),
            ..Default::default()
        };

        let submitter = ContractSubmitter::from_config(&config);
        assert!(submitter.is_err());
    }
}
