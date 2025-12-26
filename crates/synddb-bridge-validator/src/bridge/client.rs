use alloy::{
    network::EthereumWallet,
    primitives::{Address, Bytes, FixedBytes, U256},
    providers::ProviderBuilder,
    signers::local::PrivateKeySigner,
};
use anyhow::{Context, Result};

use super::types::IMessageBridge;
use crate::types::{ApplicationConfig, Message, MessageTypeConfig};

pub struct BridgeClient {
    bridge_address: Address,
    rpc_url: String,
    signer: PrivateKeySigner,
}

impl std::fmt::Debug for BridgeClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BridgeClient")
            .field("bridge_address", &self.bridge_address)
            .field("rpc_url", &self.rpc_url)
            .field("signer", &"<redacted>")
            .finish()
    }
}

impl BridgeClient {
    pub fn new(rpc_url: &str, bridge_address: Address, private_key: &str) -> Result<Self> {
        let key_bytes = private_key.strip_prefix("0x").unwrap_or(private_key);
        let signer: PrivateKeySigner = key_bytes.parse().context("Failed to parse private key")?;

        Ok(Self {
            bridge_address,
            rpc_url: rpc_url.to_string(),
            signer,
        })
    }

    pub const fn address(&self) -> Address {
        self.signer.address()
    }

    async fn read_provider(&self) -> Result<impl alloy::providers::Provider + Clone> {
        let url: reqwest::Url = self.rpc_url.parse().context("Invalid RPC URL")?;
        Ok(ProviderBuilder::new().connect_http(url))
    }

    async fn write_provider(&self) -> Result<impl alloy::providers::Provider + Clone> {
        let url: reqwest::Url = self.rpc_url.parse().context("Invalid RPC URL")?;
        let wallet = EthereumWallet::from(self.signer.clone());
        Ok(ProviderBuilder::new().wallet(wallet).connect_http(url))
    }

    pub async fn get_last_nonce(&self, domain: [u8; 32]) -> Result<u64> {
        let provider = self.read_provider().await?;
        let contract = IMessageBridge::new(self.bridge_address, provider);

        let result = contract
            .getLastNonce(FixedBytes::from(domain))
            .call()
            .await
            .context("Failed to get last nonce")?;

        Ok(result)
    }

    pub async fn get_application_config(&self, domain: [u8; 32]) -> Result<ApplicationConfig> {
        let provider = self.read_provider().await?;
        let contract = IMessageBridge::new(self.bridge_address, provider);

        let config = contract
            .getApplicationConfig(FixedBytes::from(domain))
            .call()
            .await
            .context("Failed to get application config")?;

        Ok(ApplicationConfig {
            domain,
            primary_validator: config.primaryValidator,
            expiration_seconds: config.expirationSeconds,
            require_witness_signatures: config.requireWitnessSignatures,
            active: config.active,
        })
    }

    pub async fn get_message_type_config(&self, message_type: &str) -> Result<MessageTypeConfig> {
        let provider = self.read_provider().await?;
        let contract = IMessageBridge::new(self.bridge_address, provider);

        let config = contract
            .getMessageTypeConfig(message_type.to_string())
            .call()
            .await
            .context("Failed to get message type config")?;

        Ok(MessageTypeConfig {
            message_type: message_type.to_string(),
            selector: config.selector.into(),
            target: config.target,
            schema_hash: config.schemaHash.into(),
            schema_uri: config.schemaUri,
            enabled: config.enabled,
            updated_at: config.updatedAt,
        })
    }

    pub async fn get_domain_separator(&self) -> Result<[u8; 32]> {
        let provider = self.read_provider().await?;
        let contract = IMessageBridge::new(self.bridge_address, provider);

        let result = contract
            .DOMAIN_SEPARATOR()
            .call()
            .await
            .context("Failed to get domain separator")?;

        Ok(result.into())
    }

    pub async fn initialize_message(
        &self,
        message: &Message,
        storage_ref: &str,
        value: Option<u128>,
    ) -> Result<()> {
        let provider = self.write_provider().await?;
        let contract = IMessageBridge::new(self.bridge_address, provider);

        let tx = contract
            .initializeMessage(
                FixedBytes::from(message.id),
                message.message_type.clone(),
                Bytes::from(message.calldata.clone()),
                FixedBytes::from(message.metadata_hash),
                storage_ref.to_string(),
                message.nonce,
                message.timestamp,
                FixedBytes::from(message.domain),
            )
            .value(U256::from(value.unwrap_or(0)));

        tx.send()
            .await
            .context("Failed to send tx")?
            .watch()
            .await
            .context("Failed to watch tx")?;

        Ok(())
    }

    pub async fn sign_message(&self, message_id: [u8; 32], signature: &[u8]) -> Result<()> {
        let provider = self.write_provider().await?;
        let contract = IMessageBridge::new(self.bridge_address, provider);

        contract
            .signMessage(
                FixedBytes::from(message_id),
                Bytes::from(signature.to_vec()),
            )
            .send()
            .await
            .context("Failed to send tx")?
            .watch()
            .await
            .context("Failed to watch tx")?;

        Ok(())
    }

    pub async fn initialize_and_sign(
        &self,
        message: &Message,
        storage_ref: &str,
        signature: &[u8],
        value: Option<u128>,
    ) -> Result<()> {
        let provider = self.write_provider().await?;
        let contract = IMessageBridge::new(self.bridge_address, provider);

        let tx = contract
            .initializeAndSign(
                FixedBytes::from(message.id),
                message.message_type.clone(),
                Bytes::from(message.calldata.clone()),
                FixedBytes::from(message.metadata_hash),
                storage_ref.to_string(),
                message.nonce,
                message.timestamp,
                FixedBytes::from(message.domain),
                Bytes::from(signature.to_vec()),
            )
            .value(U256::from(value.unwrap_or(0)));

        tx.send()
            .await
            .context("Failed to send tx")?
            .watch()
            .await
            .context("Failed to watch tx")?;

        Ok(())
    }

    pub async fn reject_proposal(
        &self,
        message: &Message,
        reason_hash: [u8; 32],
        reason_ref: &str,
    ) -> Result<()> {
        let provider = self.write_provider().await?;
        let contract = IMessageBridge::new(self.bridge_address, provider);

        contract
            .rejectProposal(
                FixedBytes::from(message.id),
                message.message_type.clone(),
                FixedBytes::from(message.domain),
                message.nonce,
                FixedBytes::from(reason_hash),
                reason_ref.to_string(),
            )
            .send()
            .await
            .context("Failed to send tx")?
            .watch()
            .await
            .context("Failed to watch tx")?;

        Ok(())
    }

    pub async fn reject_message(
        &self,
        message_id: [u8; 32],
        reason_hash: [u8; 32],
        reason_ref: &str,
    ) -> Result<()> {
        let provider = self.write_provider().await?;
        let contract = IMessageBridge::new(self.bridge_address, provider);

        contract
            .rejectMessage(
                FixedBytes::from(message_id),
                FixedBytes::from(reason_hash),
                reason_ref.to_string(),
            )
            .send()
            .await
            .context("Failed to send tx")?
            .watch()
            .await
            .context("Failed to watch tx")?;

        Ok(())
    }

    pub async fn get_message_stage(&self, message_id: [u8; 32]) -> Result<u8> {
        let provider = self.read_provider().await?;
        let contract = IMessageBridge::new(self.bridge_address, provider);

        let stage = contract
            .getMessageStage(FixedBytes::from(message_id))
            .call()
            .await
            .context("Failed to get message stage")?;

        Ok(stage)
    }

    pub async fn get_signature_count(&self, message_id: [u8; 32]) -> Result<u64> {
        let provider = self.read_provider().await?;
        let contract = IMessageBridge::new(self.bridge_address, provider);

        let count = contract
            .getSignatureCount(FixedBytes::from(message_id))
            .call()
            .await
            .context("Failed to get signature count")?;

        Ok(count.try_into().unwrap_or(u64::MAX))
    }

    pub async fn has_validator_signed(
        &self,
        message_id: [u8; 32],
        validator: Address,
    ) -> Result<bool> {
        let provider = self.read_provider().await?;
        let contract = IMessageBridge::new(self.bridge_address, provider);

        let signed = contract
            .hasValidatorSigned(FixedBytes::from(message_id), validator)
            .call()
            .await
            .context("Failed to check validator signature")?;

        Ok(signed)
    }

    pub async fn get_signature_threshold(&self) -> Result<u64> {
        let provider = self.read_provider().await?;
        let contract = IMessageBridge::new(self.bridge_address, provider);

        let threshold = contract
            .signatureThreshold()
            .call()
            .await
            .context("Failed to get signature threshold")?;

        Ok(threshold.try_into().unwrap_or(u64::MAX))
    }
}
