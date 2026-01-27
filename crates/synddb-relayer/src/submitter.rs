//! Contract interaction for the relayer
//!
//! Handles submitting addKey and fundKeyWithSignature transactions.

use crate::config::RelayerConfig;
use alloy::{
    network::EthereumWallet,
    primitives::{Address, Bytes, B256},
    providers::{Provider, ProviderBuilder},
    signers::local::PrivateKeySigner,
    sol,
};
use std::time::Duration;
use tracing::{debug, info, warn};
use url::Url;

// Contract interfaces
sol! {
    #[sol(rpc)]
    interface ITeeKeyManager {
        function addKey(bytes calldata publicValues, bytes calldata proofBytes) external;
        function addKeyWithSignature(
            bytes calldata publicValues,
            bytes calldata proofBytes,
            uint256 deadline,
            bytes calldata signature
        ) external;
        function isKeyValid(address publicKey) external view returns (bool);
    }

    #[sol(rpc)]
    interface IGasTreasury {
        function fundKeyWithSignature(
            address teeKey,
            uint256 deadline,
            bytes calldata signature
        ) external;
        function fundingAmount() external view returns (uint256);
    }
}

/// Submitter for relayer transactions
pub(crate) struct RelayerSubmitter {
    rpc_url: String,
    key_manager_address: Address,
    signer: PrivateKeySigner,
}

impl std::fmt::Debug for RelayerSubmitter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RelayerSubmitter")
            .field("rpc_url", &self.rpc_url)
            .field("key_manager_address", &self.key_manager_address)
            .field("relayer_address", &self.signer.address())
            .finish()
    }
}

impl RelayerSubmitter {
    /// Create from config
    pub(crate) fn from_config(config: &RelayerConfig) -> anyhow::Result<Self> {
        let key_bytes = hex::decode(config.private_key.trim_start_matches("0x"))?;
        let signer = PrivateKeySigner::from_slice(&key_bytes)?;

        Ok(Self {
            rpc_url: config.rpc_url.clone(),
            key_manager_address: config.key_manager_address,
            signer,
        })
    }

    /// Get the relayer's address
    pub(crate) const fn relayer_address(&self) -> Address {
        self.signer.address()
    }

    /// Submit addKeyWithSignature to `TeeKeyManager`
    ///
    /// Returns the transaction hash.
    pub(crate) async fn add_key_with_signature(
        &self,
        public_values: Bytes,
        proof_bytes: Bytes,
        deadline: u64,
        signature: Bytes,
    ) -> anyhow::Result<B256> {
        let wallet = EthereumWallet::from(self.signer.clone());
        let url = Url::parse(&self.rpc_url)?;
        let provider = ProviderBuilder::new().wallet(wallet).connect_http(url);

        info!(
            contract = %self.key_manager_address,
            public_values_len = public_values.len(),
            proof_bytes_len = proof_bytes.len(),
            deadline = deadline,
            "Submitting addKeyWithSignature"
        );

        let contract = ITeeKeyManager::new(self.key_manager_address, &provider);

        let tx = contract.addKeyWithSignature(
            public_values,
            proof_bytes,
            alloy::primitives::U256::from(deadline),
            signature,
        );

        let pending = tx.send().await?;
        let tx_hash = *pending.tx_hash();

        info!(tx_hash = %tx_hash, "addKeyWithSignature submitted");
        Ok(tx_hash)
    }

    /// Submit fundKeyWithSignature to a specific `GasTreasury`
    ///
    /// Returns the transaction hash.
    pub(crate) async fn fund_key_with_signature(
        &self,
        treasury_address: Address,
        tee_key: Address,
        deadline: u64,
        signature: Bytes,
    ) -> anyhow::Result<B256> {
        let wallet = EthereumWallet::from(self.signer.clone());
        let url = Url::parse(&self.rpc_url)?;
        let provider = ProviderBuilder::new().wallet(wallet).connect_http(url);

        info!(
            contract = %treasury_address,
            tee_key = %tee_key,
            deadline = deadline,
            "Submitting fundKeyWithSignature"
        );

        let contract = IGasTreasury::new(treasury_address, &provider);

        let tx = contract.fundKeyWithSignature(
            tee_key,
            alloy::primitives::U256::from(deadline),
            signature,
        );

        let pending = tx.send().await?;
        let tx_hash = *pending.tx_hash();

        info!(tx_hash = %tx_hash, "fundKeyWithSignature submitted");
        Ok(tx_hash)
    }

    /// Get the funding amount from a specific treasury
    pub(crate) async fn get_funding_amount(
        &self,
        treasury_address: Address,
    ) -> anyhow::Result<u128> {
        let url = Url::parse(&self.rpc_url)?;
        let provider = ProviderBuilder::new().connect_http(url);

        let contract = IGasTreasury::new(treasury_address, &provider);
        let amount = contract.fundingAmount().call().await?;

        Ok(amount.to::<u128>())
    }

    /// Check if a key is already registered
    pub(crate) async fn is_key_valid(&self, address: Address) -> anyhow::Result<bool> {
        let url = Url::parse(&self.rpc_url)?;
        let provider = ProviderBuilder::new().connect_http(url);

        let contract = ITeeKeyManager::new(self.key_manager_address, &provider);

        match contract.isKeyValid(address).call().await {
            Ok(_) => Ok(true),
            Err(e) => {
                let err_str = e.to_string();
                if err_str.contains("InvalidPublicKey") {
                    Ok(false)
                } else {
                    Err(e.into())
                }
            }
        }
    }

    /// Wait for transaction confirmation
    pub(crate) async fn wait_for_confirmation(&self, tx_hash: B256) -> anyhow::Result<()> {
        let url = Url::parse(&self.rpc_url)?;
        let provider = ProviderBuilder::new().connect_http(url);

        let poll_interval = Duration::from_secs(2);
        let timeout = Duration::from_secs(120);
        let start = std::time::Instant::now();

        loop {
            if start.elapsed() > timeout {
                anyhow::bail!("Timeout waiting for tx confirmation: {}", tx_hash);
            }

            match provider.get_transaction_receipt(tx_hash).await {
                Ok(Some(receipt)) => {
                    if receipt.status() {
                        info!(tx_hash = %tx_hash, "Transaction confirmed");
                        return Ok(());
                    }
                    anyhow::bail!("Transaction reverted: {}", tx_hash);
                }
                Ok(None) => {
                    debug!(tx_hash = %tx_hash, "Transaction pending...");
                }
                Err(e) => {
                    warn!(error = %e, "Error checking receipt");
                }
            }

            tokio::time::sleep(poll_interval).await;
        }
    }
}
