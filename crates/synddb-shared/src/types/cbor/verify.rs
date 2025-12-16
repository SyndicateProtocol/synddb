//! Consolidated secp256k1 signature verification for COSE messages
//!
//! This module provides signature verification using k256's `DigestVerifier` trait
//! with Keccak256 hashing for Ethereum compatibility.
//!
//! See [README.md](./README.md) for details on why this approach is used.

use k256::ecdsa::{signature::DigestVerifier, Signature, VerifyingKey};
use sha3::{Digest, Keccak256};
use crate::types::cbor::error::CborError;

/// Verify a secp256k1 ECDSA signature using keccak256 hashing.
///
/// This function uses `DigestVerifier::verify_digest` with Keccak256, which is
/// the idiomatic k256 approach for custom digest verification.
///
/// # Arguments
/// - `data`: The raw data that was signed (will be hashed with keccak256)
/// - `signature`: 64-byte signature (r || s format, as used by COSE)
/// - `pubkey`: 64-byte uncompressed public key (without the 0x04 prefix)
///
/// # Example
/// ```rust,ignore
/// // Verify a COSE Sig_structure
/// let sig_structure = cose.tbs_data(&[]);
/// verify_secp256k1(&sig_structure, &signature, &pubkey)?;
/// ```
pub fn verify_secp256k1(
    data: &[u8],
    signature: &[u8; 64],
    pubkey: &[u8; 64],
) -> Result<(), CborError> {
    // Reconstruct the uncompressed SEC1 public key (0x04 || x || y)
    let mut sec1_pubkey = [0u8; 65];
    sec1_pubkey[0] = 0x04;
    sec1_pubkey[1..].copy_from_slice(pubkey);

    // Parse the public key using k256's SEC1 parsing
    let verifying_key = VerifyingKey::from_sec1_bytes(&sec1_pubkey)
        .map_err(|e| CborError::SignatureVerification(format!("Invalid public key: {e}")))?;

    // Parse the 64-byte (r || s) signature
    let sig = Signature::from_slice(signature)
        .map_err(|e| CborError::SignatureVerification(format!("Invalid signature: {e}")))?;

    // Create keccak256 digest of the data
    let digest = Keccak256::new_with_prefix(data);

    // Verify using DigestVerifier trait (type-safe, not hazmat)
    verifying_key
        .verify_digest(digest, &sig)
        .map_err(|_| CborError::SignatureVerification("Signature verification failed".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::keccak256;
    use alloy::signers::{local::PrivateKeySigner, SignerSync};

    /// Test private key (well-known test key, do not use in production)
    const TEST_PRIVATE_KEY: &str =
        "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

    #[test]
    fn test_verify_valid_signature() {
        let signer: PrivateKeySigner = TEST_PRIVATE_KEY.parse().unwrap();
        let pubkey = signer.public_key().0;

        let data = b"test message to sign";

        // Sign: hash with keccak256, then sign the hash
        let hash = keccak256(data);
        let sig = signer
            .sign_hash_sync(&hash.into())
            .expect("signing should succeed");

        // Extract r and s (64 bytes total, no v)
        let mut signature = [0u8; 64];
        signature[..32].copy_from_slice(&sig.r().to_be_bytes::<32>());
        signature[32..].copy_from_slice(&sig.s().to_be_bytes::<32>());

        // Verify using our function (which also hashes with keccak256 internally)
        let result = verify_secp256k1(data, &signature, &pubkey);
        assert!(result.is_ok(), "Valid signature should verify: {result:?}");
    }

    #[test]
    fn test_verify_wrong_data_fails() {
        let signer: PrivateKeySigner = TEST_PRIVATE_KEY.parse().unwrap();
        let pubkey = signer.public_key().0;

        let data = b"original message";
        let wrong_data = b"different message";

        // Sign the original data
        let hash = keccak256(data);
        let sig = signer.sign_hash_sync(&hash.into()).unwrap();

        let mut signature = [0u8; 64];
        signature[..32].copy_from_slice(&sig.r().to_be_bytes::<32>());
        signature[32..].copy_from_slice(&sig.s().to_be_bytes::<32>());

        // Verify against wrong data should fail
        let result = verify_secp256k1(wrong_data, &signature, &pubkey);
        assert!(result.is_err(), "Wrong data should fail verification");
    }

    #[test]
    fn test_verify_invalid_pubkey_zeros() {
        let data = b"test data";
        let signature = [0u8; 64];
        let pubkey = [0u8; 64]; // Invalid all-zeros pubkey

        let result = verify_secp256k1(data, &signature, &pubkey);
        assert!(result.is_err());
        assert!(matches!(result, Err(CborError::SignatureVerification(_))));
    }

    #[test]
    fn test_verify_invalid_pubkey_ones() {
        let data = b"test data";
        let signature = [0xffu8; 64];
        let pubkey = [0x01u8; 64]; // Invalid pubkey

        let result = verify_secp256k1(data, &signature, &pubkey);
        assert!(result.is_err());
    }
}
