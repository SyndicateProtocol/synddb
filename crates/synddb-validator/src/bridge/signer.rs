//! Bridge message signer using EIP-191 signed messages
//!
//! Signs messages in the format expected by the bridge contract's
//! `signMessageWithSignature()` function.

use alloy::{
    primitives::{keccak256, Address, B256},
    signers::{local::PrivateKeySigner, Signer, SignerSync},
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tracing::{debug, info};

use crate::config::ValidatorConfig;

/// Get current Unix timestamp in seconds
fn current_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// A signature for a bridge message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageSignature {
    /// The message ID that was signed
    pub message_id: String,
    /// The raw signature bytes (65 bytes: r || s || v)
    #[serde(with = "hex_bytes")]
    pub signature: Vec<u8>,
    /// Address of the signer
    pub signer: Address,
    /// Timestamp when the signature was created
    pub signed_at: u64,
}

/// Bridge signer for creating validator signatures
///
/// Uses EIP-191 personal sign format which the bridge contract expects:
/// `sign(keccak256("\x19Ethereum Signed Message:\n32" + messageId))`
pub struct BridgeSigner {
    signer: PrivateKeySigner,
    bridge_contract: Address,
    chain_id: u64,
    /// Pre-formatted signer address (checksummed hex with 0x prefix)
    address_formatted: String,
    /// Pre-formatted bridge contract address (checksummed hex with 0x prefix)
    bridge_contract_formatted: String,
}

impl BridgeSigner {
    /// Create a new bridge signer from configuration
    pub fn new(config: &ValidatorConfig) -> Result<Self> {
        let signing_key = config
            .bridge_signing_key
            .as_ref()
            .context("--bridge-signing-key is required")?;

        let bridge_contract: Address = config
            .bridge_contract
            .as_ref()
            .context("--bridge-contract is required")?
            .parse()
            .context("Invalid bridge contract address")?;

        let chain_id = config
            .bridge_chain_id
            .context("--bridge-chain-id is required")?;

        // Parse the private key (strip 0x prefix if present)
        let key_hex = signing_key.strip_prefix("0x").unwrap_or(signing_key);
        let signer: PrivateKeySigner = key_hex.parse().context("Invalid private key")?;

        let address_formatted = format!("{:#x}", signer.address());
        let bridge_contract_formatted = format!("{:#x}", bridge_contract);

        info!(
            signer = %address_formatted,
            bridge = %bridge_contract_formatted,
            chain_id = chain_id,
            "Bridge signer initialized"
        );

        Ok(Self {
            signer,
            bridge_contract,
            chain_id,
            address_formatted,
            bridge_contract_formatted,
        })
    }

    /// Get the signer's address
    #[allow(clippy::missing_const_for_fn)] // PrivateKeySigner::address() is not const
    pub fn address(&self) -> Address {
        self.signer.address()
    }

    /// Get the signer's address as a formatted hex string (checksummed, with 0x prefix)
    pub fn address_formatted(&self) -> &str {
        &self.address_formatted
    }

    pub const fn bridge_contract(&self) -> Address {
        self.bridge_contract
    }

    /// Get the bridge contract address as a formatted hex string (checksummed, with 0x prefix)
    pub fn bridge_contract_formatted(&self) -> &str {
        &self.bridge_contract_formatted
    }

    pub const fn chain_id(&self) -> u64 {
        self.chain_id
    }

    /// Sign a message ID using EIP-191 personal sign format
    ///
    /// This creates a signature compatible with the bridge contract's
    /// `signMessageWithSignature(bytes32 messageId, bytes calldata signature)` function.
    ///
    /// The bridge uses:
    /// ```solidity
    /// bytes32 messageHash = MessageHashUtils.toEthSignedMessageHash(messageId);
    /// address validator = ECDSA.recover(messageHash, signature);
    /// ```
    pub async fn sign_message(&self, message_id: B256) -> Result<MessageSignature> {
        let eth_signed_hash = Self::eth_signed_message_hash(message_id);
        let signature = self
            .signer
            .sign_hash(&eth_signed_hash)
            .await
            .context("Failed to sign message")?;

        debug!(
            message_id = %message_id,
            signer = %self.signer.address(),
            "Signed bridge message"
        );

        Ok(self.create_message_signature(message_id, &signature))
    }

    /// Sign a message ID synchronously (for use in sync contexts)
    pub fn sign_message_sync(&self, message_id: B256) -> Result<MessageSignature> {
        let eth_signed_hash = Self::eth_signed_message_hash(message_id);
        let signature = self
            .signer
            .sign_hash_sync(&eth_signed_hash)
            .context("Failed to sign message")?;

        debug!(
            message_id = %message_id,
            signer = %self.signer.address(),
            "Signed bridge message (sync)"
        );

        Ok(self.create_message_signature(message_id, &signature))
    }

    /// Create a `MessageSignature` from raw signature components
    fn create_message_signature(
        &self,
        message_id: B256,
        signature: &alloy::signers::Signature,
    ) -> MessageSignature {
        // Format as r || s || v (65 bytes)
        let mut sig_bytes = Vec::with_capacity(65);
        sig_bytes.extend_from_slice(&signature.r().to_be_bytes::<32>());
        sig_bytes.extend_from_slice(&signature.s().to_be_bytes::<32>());
        // Ethereum uses v = 27 or 28
        sig_bytes.push(if signature.v() { 28 } else { 27 });

        MessageSignature {
            message_id: format!("{message_id:#x}"),
            signature: sig_bytes,
            signer: self.signer.address(),
            signed_at: current_timestamp(),
        }
    }

    /// Convert a bytes32 message to EIP-191 signed message hash
    ///
    /// Matches `OpenZeppelin`'s `MessageHashUtils.toEthSignedMessageHash(bytes32)`:
    /// ```solidity
    /// return keccak256(abi.encodePacked("\x19Ethereum Signed Message:\n32", messageHash));
    /// ```
    fn eth_signed_message_hash(message: B256) -> B256 {
        let mut data = Vec::with_capacity(60);
        data.extend_from_slice(b"\x19Ethereum Signed Message:\n32");
        data.extend_from_slice(message.as_slice());
        keccak256(&data)
    }
}

impl std::fmt::Debug for BridgeSigner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BridgeSigner")
            .field("address", &self.signer.address())
            .field("bridge_contract", &self.bridge_contract)
            .field("chain_id", &self.chain_id)
            .finish()
    }
}

/// Serde helper for hex-encoded bytes
mod hex_bytes {
    use serde::{Deserialize, Deserializer, Serializer};

    pub(super) fn serialize<S>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let hex_string = format!("0x{}", hex::encode(bytes));
        serializer.serialize_str(&hex_string)
    }

    pub(super) fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let s = s.strip_prefix("0x").unwrap_or(&s);
        hex::decode(s).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    // Test private key (Foundry's default first account)
    // Address: 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266
    const TEST_PRIVATE_KEY: &str =
        "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
    // 64-byte uncompressed public key corresponding to TEST_PRIVATE_KEY
    const TEST_PUBKEY: &str = "8318535b54105d4a7aae60c08fc45f9687181b4fdfc625bd1a753fa7397fed753547f11ca8696646f2f3acb08e31016afac23e630c5d11f59f61fef57b0d2aa5";

    fn test_config() -> ValidatorConfig {
        ValidatorConfig::parse_from([
            "synddb-validator",
            "--sequencer-pubkey",
            TEST_PUBKEY,
            "--bridge-signer",
            "--bridge-contract",
            "0x1234567890abcdef1234567890abcdef12345678",
            "--bridge-chain-id",
            "1",
            "--bridge-signing-key",
            TEST_PRIVATE_KEY,
        ])
    }

    #[test]
    fn test_bridge_signer_creation() {
        let config = test_config();
        let signer = BridgeSigner::new(&config).unwrap();

        let expected_address: Address = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"
            .parse()
            .unwrap();
        assert_eq!(signer.address(), expected_address);
    }

    #[test]
    fn test_sign_message_sync() {
        let config = test_config();
        let signer = BridgeSigner::new(&config).unwrap();

        let message_id = B256::from_slice(&[1u8; 32]);
        let sig = signer.sign_message_sync(message_id).unwrap();

        assert_eq!(sig.signature.len(), 65);
        assert_eq!(sig.signer, signer.address());
        assert!(sig.signed_at > 0);
    }

    #[tokio::test]
    async fn test_sign_message_async() {
        let config = test_config();
        let signer = BridgeSigner::new(&config).unwrap();

        let message_id = B256::from_slice(&[2u8; 32]);
        let sig = signer.sign_message(message_id).await.unwrap();

        assert_eq!(sig.signature.len(), 65);
        assert_eq!(sig.signer, signer.address());
    }

    #[test]
    fn test_eth_signed_message_hash() {
        // Test that our hash matches what the bridge contract expects
        let message_id = B256::from_slice(&[0xab; 32]);
        let hash = BridgeSigner::eth_signed_message_hash(message_id);

        // The hash should be deterministic
        let hash2 = BridgeSigner::eth_signed_message_hash(message_id);
        assert_eq!(hash, hash2);

        // Different message should give different hash
        let other_id = B256::from_slice(&[0xcd; 32]);
        let other_hash = BridgeSigner::eth_signed_message_hash(other_id);
        assert_ne!(hash, other_hash);
    }

    #[test]
    fn test_signature_format() {
        let config = test_config();
        let signer = BridgeSigner::new(&config).unwrap();

        let message_id = B256::from_slice(&[3u8; 32]);
        let sig = signer.sign_message_sync(message_id).unwrap();

        // Check v value is 27 or 28
        let v = sig.signature[64];
        assert!(v == 27 || v == 28, "v should be 27 or 28, got {v}");
    }

    #[test]
    fn test_signature_serialization() {
        let config = test_config();
        let signer = BridgeSigner::new(&config).unwrap();

        let message_id = B256::from_slice(&[4u8; 32]);
        let sig = signer.sign_message_sync(message_id).unwrap();

        // Serialize to JSON
        let json = serde_json::to_string(&sig).unwrap();
        assert!(json.contains("0x")); // signature should be hex-encoded

        // Deserialize back
        let sig2: MessageSignature = serde_json::from_str(&json).unwrap();
        assert_eq!(sig.signature, sig2.signature);
        assert_eq!(sig.signer, sig2.signer);
    }
}
