use alloy::{
    primitives::Address,
    signers::{local::PrivateKeySigner, Signer},
};
use anyhow::{Context, Result};

use crate::types::Message;

use super::eip712::{compute_digest, compute_domain_separator, compute_struct_hash};

pub struct MessageSigner {
    signer: PrivateKeySigner,
    domain_separator: [u8; 32],
}

impl std::fmt::Debug for MessageSigner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MessageSigner")
            .field("address", &self.signer.address())
            .field("signer", &"<redacted>")
            .finish()
    }
}

impl MessageSigner {
    pub fn new(private_key: &str, chain_id: u64, bridge_address: Address) -> Result<Self> {
        let key_bytes = private_key.strip_prefix("0x").unwrap_or(private_key);
        let signer: PrivateKeySigner = key_bytes.parse().context("Failed to parse private key")?;

        let domain_separator = compute_domain_separator(chain_id, bridge_address);

        Ok(Self {
            signer,
            domain_separator,
        })
    }

    pub const fn address(&self) -> Address {
        self.signer.address()
    }

    pub async fn sign_message(&self, message: &Message) -> Result<Vec<u8>> {
        let struct_hash = compute_struct_hash(message);
        let digest = compute_digest(&self.domain_separator, &struct_hash);

        let signature = self
            .signer
            .sign_hash(&digest.into())
            .await
            .context("Failed to sign message")?;

        Ok(signature.as_bytes().to_vec())
    }

    pub const fn domain_separator(&self) -> &[u8; 32] {
        &self.domain_separator
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_sign_message() {
        let private_key = "0x0000000000000000000000000000000000000000000000000000000000000001";
        let chain_id = 1u64;
        let bridge_address = Address::ZERO;

        let signer = MessageSigner::new(private_key, chain_id, bridge_address).unwrap();

        let message = Message {
            id: [0u8; 32],
            message_type: "setValue(uint256)".to_string(),
            calldata: vec![0u8; 32],
            metadata: serde_json::Value::Null,
            metadata_hash: [0u8; 32],
            nonce: 1,
            timestamp: 1234567890,
            domain: [0u8; 32],
            value: None,
        };

        let signature = signer.sign_message(&message).await.unwrap();
        assert_eq!(signature.len(), 65); // r(32) + s(32) + v(1)
    }

    #[test]
    fn test_signer_address() {
        let private_key = "0x0000000000000000000000000000000000000000000000000000000000000001";
        let signer = MessageSigner::new(private_key, 1, Address::ZERO).unwrap();

        let address = signer.address();
        assert_ne!(address, Address::ZERO);
    }
}
