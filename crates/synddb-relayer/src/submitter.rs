//! Contract interaction for the relayer
//!
//! Handles submitting key registration transactions to the Bridge contract.

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

// Bridge contract interface for key registration
sol! {
    #[sol(rpc)]
    interface IBridge {
        function registerSequencerKeyWithSignature(
            bytes calldata publicValues,
            bytes calldata proofBytes,
            uint256 deadline,
            bytes calldata signature
        ) external returns (address publicKey);

        function registerValidatorKeyWithSignature(
            bytes calldata publicValues,
            bytes calldata proofBytes,
            uint256 deadline,
            bytes calldata signature
        ) external returns (address publicKey);

        function teeKeyManager() external view returns (address);
    }

    #[sol(rpc)]
    interface ITeeKeyManager {
        function isSequencerKeyValid(address publicKey) external view returns (bool);
        function isValidatorKeyValid(address publicKey) external view returns (bool);
    }
}

/// Submitter for relayer transactions
pub(crate) struct RelayerSubmitter {
    rpc_url: String,
    bridge_address: Address,
    tee_key_manager_address: Address,
    signer: PrivateKeySigner,
}

impl std::fmt::Debug for RelayerSubmitter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RelayerSubmitter")
            .field("rpc_url", &self.rpc_url)
            .field("bridge_address", &self.bridge_address)
            .field("relayer_address", &self.signer.address())
            .finish()
    }
}

impl RelayerSubmitter {
    /// Create from config, fetching `TeeKeyManager` address from Bridge contract
    pub(crate) async fn from_config(config: &RelayerConfig) -> anyhow::Result<Self> {
        let key_bytes = hex::decode(config.private_key.trim_start_matches("0x"))?;
        let signer = PrivateKeySigner::from_slice(&key_bytes)?;

        // Fetch TeeKeyManager address from Bridge contract
        let url = Url::parse(&config.rpc_url)?;
        let provider = ProviderBuilder::new().connect_http(url);
        let bridge = IBridge::new(config.bridge_address, &provider);
        let tee_key_manager_address = Address::from(bridge.teeKeyManager().call().await?.0);

        info!(
            bridge = %config.bridge_address,
            tee_key_manager = %tee_key_manager_address,
            "Fetched TeeKeyManager address from Bridge"
        );

        Ok(Self {
            rpc_url: config.rpc_url.clone(),
            bridge_address: config.bridge_address,
            tee_key_manager_address,
            signer,
        })
    }

    /// Register a sequencer key via signature
    pub(crate) async fn register_sequencer_key(
        &self,
        public_values: Vec<u8>,
        proof_bytes: Vec<u8>,
        deadline: u64,
        signature: Vec<u8>,
    ) -> anyhow::Result<B256> {
        let wallet = EthereumWallet::from(self.signer.clone());
        let url = Url::parse(&self.rpc_url)?;
        let provider = ProviderBuilder::new().wallet(wallet).connect_http(url);

        info!(
            contract = %self.bridge_address,
            public_values_len = public_values.len(),
            proof_bytes_len = proof_bytes.len(),
            deadline = deadline,
            "Submitting registerSequencerKeyWithSignature"
        );

        let contract = IBridge::new(self.bridge_address, &provider);

        let tx = contract.registerSequencerKeyWithSignature(
            Bytes::from(public_values),
            Bytes::from(proof_bytes),
            alloy::primitives::U256::from(deadline),
            Bytes::from(signature),
        );

        let pending = tx.send().await?;
        let tx_hash = *pending.tx_hash();

        info!(tx_hash = %tx_hash, "registerSequencerKeyWithSignature submitted");
        Ok(tx_hash)
    }

    /// Register a validator key via signature
    pub(crate) async fn register_validator_key(
        &self,
        public_values: Vec<u8>,
        proof_bytes: Vec<u8>,
        deadline: u64,
        signature: Vec<u8>,
    ) -> anyhow::Result<B256> {
        let wallet = EthereumWallet::from(self.signer.clone());
        let url = Url::parse(&self.rpc_url)?;
        let provider = ProviderBuilder::new().wallet(wallet).connect_http(url);

        info!(
            contract = %self.bridge_address,
            public_values_len = public_values.len(),
            proof_bytes_len = proof_bytes.len(),
            deadline = deadline,
            "Submitting registerValidatorKeyWithSignature"
        );

        let contract = IBridge::new(self.bridge_address, &provider);

        let tx = contract.registerValidatorKeyWithSignature(
            Bytes::from(public_values),
            Bytes::from(proof_bytes),
            alloy::primitives::U256::from(deadline),
            Bytes::from(signature),
        );

        let pending = tx.send().await?;
        let tx_hash = *pending.tx_hash();

        info!(tx_hash = %tx_hash, "registerValidatorKeyWithSignature submitted");
        Ok(tx_hash)
    }

    /// Check if a sequencer key is valid
    pub(crate) async fn is_sequencer_key_valid(&self, address: Address) -> anyhow::Result<bool> {
        let url = Url::parse(&self.rpc_url)?;
        let provider = ProviderBuilder::new().connect_http(url);

        let contract = ITeeKeyManager::new(self.tee_key_manager_address, &provider);

        match contract.isSequencerKeyValid(address).call().await {
            Ok(_) => Ok(true),
            Err(e) => {
                let err_str = e.to_string();
                // InvalidPublicKey error selector is 0xffc44e88
                if err_str.contains("InvalidPublicKey") || err_str.contains("0xffc44e88") {
                    Ok(false)
                } else {
                    Err(e.into())
                }
            }
        }
    }

    /// Check if a validator key is valid
    pub(crate) async fn is_validator_key_valid(&self, address: Address) -> anyhow::Result<bool> {
        let url = Url::parse(&self.rpc_url)?;
        let provider = ProviderBuilder::new().connect_http(url);

        let contract = ITeeKeyManager::new(self.tee_key_manager_address, &provider);

        match contract.isValidatorKeyValid(address).call().await {
            Ok(_) => Ok(true),
            Err(e) => {
                let err_str = e.to_string();
                // InvalidPublicKey error selector is 0xffc44e88
                if err_str.contains("InvalidPublicKey") || err_str.contains("0xffc44e88") {
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
