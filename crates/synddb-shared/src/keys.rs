//! EVM key management for sequencers and validators
//!
//! Provides secure secp256k1 key generation for signing EVM-compatible messages.
//! Keys are generated fresh at startup using OS-level secure randomness and are
//! never logged or exposed outside the enclave.
//!
//! # Security Model
//!
//! - Keys are generated at startup using secure OS randomness
//! - Private key material is NEVER logged or exposed
//! - Only the public key/address is exposed for external verification
//! - No persistence - keys are ephemeral to each instance
//!
//! # Usage
//!
//! ```rust
//! use synddb_shared::keys::EvmKeyManager;
//!
//! let key_manager = EvmKeyManager::generate();
//! println!("Signer address: {}", key_manager.address());
//! println!("Public key: 0x{}", hex::encode(key_manager.public_key()));
//! ```

use alloy::{
    primitives::{Address, B256, B512},
    signers::{local::PrivateKeySigner, Signer, SignerSync},
};
use thiserror::Error;

/// Errors that can occur during key operations
#[derive(Debug, Error)]
pub enum KeyError {
    #[error("Signing failed: {0}")]
    SigningFailed(String),
}

/// EVM key manager for secure signing operations
///
/// Generates and holds a secp256k1 private key for EVM-compatible signatures.
/// The private key is generated using OS-level secure randomness and is
/// never logged or serialized.
pub struct EvmKeyManager {
    signer: PrivateKeySigner,
}

impl EvmKeyManager {
    /// Generate a new key manager with a fresh random key
    ///
    /// Uses OS-level secure randomness (via `getrandom`) to generate
    /// a cryptographically secure secp256k1 private key.
    ///
    /// # Panics
    ///
    /// Panics if the OS cannot provide secure random bytes (extremely rare).
    #[must_use]
    pub fn generate() -> Self {
        let signer = PrivateKeySigner::random();
        Self { signer }
    }

    /// Get the Ethereum address derived from the signing key
    #[must_use]
    pub fn address(&self) -> Address {
        self.signer.address()
    }

    /// Get the 64-byte uncompressed public key (without 0x04 prefix)
    ///
    /// This is the raw secp256k1 public key: 32 bytes X coordinate || 32 bytes Y coordinate.
    /// Used for signature verification in COSE messages.
    #[must_use]
    pub fn public_key(&self) -> [u8; 64] {
        self.signer.public_key().0
    }

    /// Get the public key as a B512
    #[must_use]
    pub fn public_key_b512(&self) -> B512 {
        self.signer.public_key()
    }

    /// Sign a 32-byte hash synchronously
    ///
    /// Returns the full alloy Signature which contains r, s, and v.
    pub fn sign_hash_sync(&self, hash: &B256) -> Result<alloy::signers::Signature, KeyError> {
        self.signer
            .sign_hash_sync(hash)
            .map_err(|e| KeyError::SigningFailed(e.to_string()))
    }

    /// Sign a 32-byte hash asynchronously
    ///
    /// Returns the full alloy Signature which contains r, s, and v.
    pub async fn sign_hash(&self, hash: &B256) -> Result<alloy::signers::Signature, KeyError> {
        self.signer
            .sign_hash(hash)
            .await
            .map_err(|e| KeyError::SigningFailed(e.to_string()))
    }

    /// Sign a raw 32-byte array synchronously (convenience method)
    pub fn sign_raw_sync(&self, hash: &[u8; 32]) -> Result<alloy::signers::Signature, KeyError> {
        self.sign_hash_sync(&B256::from(*hash))
    }

    /// Compute SHA-256 hash of data (for content hashing)
    #[must_use]
    pub fn compute_content_hash(data: &[u8]) -> [u8; 32] {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(data);
        let result = hasher.finalize();
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&result);
        arr
    }
}

impl std::fmt::Debug for EvmKeyManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // NEVER include private key in debug output
        f.debug_struct("EvmKeyManager")
            .field("address", &self.address())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_unique_keys() {
        let km1 = EvmKeyManager::generate();
        let km2 = EvmKeyManager::generate();

        // Each generation should produce a unique key
        assert_ne!(km1.address(), km2.address());
        assert_ne!(km1.public_key(), km2.public_key());
    }

    #[test]
    fn test_public_key_length() {
        let km = EvmKeyManager::generate();
        assert_eq!(km.public_key().len(), 64);
    }

    #[test]
    fn test_sign_hash_sync() {
        let km = EvmKeyManager::generate();
        let hash = B256::from([0x42u8; 32]);

        let sig = km.sign_hash_sync(&hash).unwrap();

        // Check r and s are populated
        assert_ne!(sig.r(), alloy::primitives::U256::ZERO);
        assert_ne!(sig.s(), alloy::primitives::U256::ZERO);
    }

    #[test]
    fn test_sign_deterministic() {
        let km = EvmKeyManager::generate();
        let hash = B256::from([0x42u8; 32]);

        let sig1 = km.sign_hash_sync(&hash).unwrap();
        let sig2 = km.sign_hash_sync(&hash).unwrap();

        // Same input should produce same signature (deterministic signing)
        assert_eq!(sig1.r(), sig2.r());
        assert_eq!(sig1.s(), sig2.s());
    }

    #[test]
    fn test_content_hash() {
        let data = b"test content";
        let hash = EvmKeyManager::compute_content_hash(data);
        assert_eq!(hash.len(), 32);

        // Same input should produce same hash
        let hash2 = EvmKeyManager::compute_content_hash(data);
        assert_eq!(hash, hash2);
    }

    #[test]
    fn test_debug_does_not_leak_key() {
        let km = EvmKeyManager::generate();
        let debug_str = format!("{:?}", km);

        // Debug output should NOT contain private key material
        // It should only show the address
        assert!(debug_str.contains("EvmKeyManager"));
        assert!(debug_str.contains("address"));
    }

    #[tokio::test]
    async fn test_sign_hash_async() {
        let km = EvmKeyManager::generate();
        let hash = B256::from([0x42u8; 32]);

        let sig = km.sign_hash(&hash).await.unwrap();

        assert_ne!(sig.r(), alloy::primitives::U256::ZERO);
        assert_ne!(sig.s(), alloy::primitives::U256::ZERO);
    }
}
