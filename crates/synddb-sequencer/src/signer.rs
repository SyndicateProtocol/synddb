//! Message signing using secp256k1 (Ethereum-compatible)
//!
//! Provides 64-byte COSE-compatible signatures (r || s format, without recovery ID).
//! Used by the inbox for signing individual messages and by the batcher for
//! signing batches.

use alloy::{
    primitives::{Address, B256, B512},
    signers::{local::PrivateKeySigner, SignerSync},
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SignerError {
    #[error("Invalid private key: {0}")]
    InvalidKey(String),

    #[error("Signing failed: {0}")]
    SigningFailed(String),
}

/// Ethereum-compatible message signer
#[derive(Debug, Clone)]
pub struct MessageSigner {
    signer: PrivateKeySigner,
}

impl MessageSigner {
    /// Create a new signer from a hex-encoded private key
    ///
    /// The key should be 64 hex characters (32 bytes), with or without "0x" prefix.
    pub fn new(private_key_hex: &str) -> Result<Self, SignerError> {
        // Strip 0x prefix if present
        let key_hex = private_key_hex
            .strip_prefix("0x")
            .unwrap_or(private_key_hex);

        let signer: PrivateKeySigner = key_hex
            .parse()
            .map_err(|e| SignerError::InvalidKey(format!("{e}")))?;

        Ok(Self { signer })
    }

    /// Get the Ethereum address derived from the signing key
    pub const fn address(&self) -> Address {
        self.signer.address()
    }

    /// Get the 64-byte uncompressed public key (without 0x04 prefix)
    ///
    /// This is the raw secp256k1 public key: 32 bytes X coordinate || 32 bytes Y coordinate.
    /// Used for signature verification in COSE messages.
    pub fn public_key(&self) -> [u8; 64] {
        self.signer.public_key().0
    }

    /// Get the public key as a B512
    pub fn public_key_b512(&self) -> B512 {
        self.signer.public_key()
    }

    /// Sign a 32-byte hash synchronously
    ///
    /// Returns the full alloy Signature which contains r, s, and v.
    /// Use this for COSE signing where only r || s (64 bytes) are needed.
    pub fn sign_raw_sync(&self, hash: &[u8; 32]) -> Result<alloy::signers::Signature, SignerError> {
        let hash = B256::from(*hash);
        self.signer
            .sign_hash_sync(&hash)
            .map_err(|e| SignerError::SigningFailed(e.to_string()))
    }

    /// Compute SHA-256 content hash from serialized messages
    pub fn compute_content_hash(messages_bytes: &[u8]) -> [u8; 32] {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(messages_bytes);
        let result = hasher.finalize();
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&result);
        arr
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Test private key (DO NOT use in production!)
    // Address: 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266
    const TEST_PRIVATE_KEY: &str =
        "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

    #[test]
    fn test_signer_creation() {
        let signer = MessageSigner::new(TEST_PRIVATE_KEY).unwrap();
        let address = signer.address();

        // Verify the address matches expected for this test key (case-insensitive)
        assert_eq!(
            format!("{address:?}").to_lowercase(),
            "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266".to_lowercase()
        );
    }

    #[test]
    fn test_signer_with_0x_prefix() {
        let signer = MessageSigner::new(&format!("0x{TEST_PRIVATE_KEY}")).unwrap();
        assert_eq!(
            format!("{:?}", signer.address()).to_lowercase(),
            "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266".to_lowercase()
        );
    }

    #[test]
    fn test_invalid_key() {
        assert!(MessageSigner::new("invalid").is_err());
        assert!(MessageSigner::new("").is_err());
        assert!(MessageSigner::new("0x").is_err());
    }

    #[test]
    fn test_public_key_length() {
        let signer = MessageSigner::new(TEST_PRIVATE_KEY).unwrap();
        let pubkey = signer.public_key();
        assert_eq!(pubkey.len(), 64);
    }

    #[test]
    fn test_sign_raw_sync() {
        let signer = MessageSigner::new(TEST_PRIVATE_KEY).unwrap();
        let hash = [0x42u8; 32];

        let sig = signer.sign_raw_sync(&hash).unwrap();

        // Check r and s are populated
        assert_ne!(sig.r(), alloy::primitives::U256::ZERO);
        assert_ne!(sig.s(), alloy::primitives::U256::ZERO);
    }

    #[test]
    fn test_sign_raw_sync_deterministic() {
        let signer = MessageSigner::new(TEST_PRIVATE_KEY).unwrap();
        let hash = [0x42u8; 32];

        let sig1 = signer.sign_raw_sync(&hash).unwrap();
        let sig2 = signer.sign_raw_sync(&hash).unwrap();

        // Same input should produce same signature (deterministic signing)
        assert_eq!(sig1.r(), sig2.r());
        assert_eq!(sig1.s(), sig2.s());
    }

    #[test]
    fn test_content_hash() {
        let data = b"test content";
        let hash = MessageSigner::compute_content_hash(data);
        assert_eq!(hash.len(), 32);

        // Same input should produce same hash
        let hash2 = MessageSigner::compute_content_hash(data);
        assert_eq!(hash, hash2);
    }
}
