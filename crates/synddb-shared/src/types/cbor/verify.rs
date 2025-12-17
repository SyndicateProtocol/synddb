//! Consolidated secp256k1 signature verification for COSE messages
//!
//! This module provides signature verification using k256's `DigestVerifier` trait
//! with Keccak256 hashing for Ethereum compatibility.
//!
//! See [README.md](./README.md) for details on why this approach is used.

use crate::types::cbor::error::CborError;
use k256::ecdsa::{signature::DigestVerifier, Signature, VerifyingKey};
use sha3::{Digest, Keccak256};

/// Convert a 64-byte uncompressed public key (without 0x04 prefix) to a `VerifyingKey`.
///
/// This validates that the bytes represent a valid point on the secp256k1 curve.
pub fn verifying_key_from_bytes(pubkey: &[u8; 64]) -> Result<VerifyingKey, CborError> {
    // Reconstruct the uncompressed SEC1 public key (0x04 || x || y)
    let mut sec1_pubkey = [0u8; 65];
    sec1_pubkey[0] = 0x04;
    sec1_pubkey[1..].copy_from_slice(pubkey);

    VerifyingKey::from_sec1_bytes(&sec1_pubkey)
        .map_err(|e| CborError::SignatureVerification(format!("Invalid public key: {e}")))
}

/// Convert a `VerifyingKey` to a 64-byte uncompressed public key (without 0x04 prefix).
pub fn verifying_key_to_bytes(key: &VerifyingKey) -> [u8; 64] {
    let sec1 = key.to_encoded_point(false); // false = uncompressed
    let bytes = sec1.as_bytes();
    // SEC1 uncompressed is 65 bytes: 0x04 || x || y
    let mut result = [0u8; 64];
    result.copy_from_slice(&bytes[1..65]);
    result
}

/// Verify a secp256k1 ECDSA signature using keccak256 hashing.
///
/// This function uses `DigestVerifier::verify_digest` with Keccak256, which is
/// the idiomatic k256 approach for custom digest verification.
///
/// # Arguments
/// - `data`: The raw data that was signed (will be hashed with keccak256)
/// - `signature`: The ECDSA signature (r || s format)
/// - `verifying_key`: The public key to verify against
///
/// # Example
/// ```rust,ignore
/// // Verify a COSE Sig_structure
/// let sig_structure = cose.tbs_data(&[]);
/// let key = verifying_key_from_bytes(&pubkey_bytes)?;
/// verify_secp256k1(&sig_structure, &signature, &key)?;
/// ```
pub fn verify_secp256k1(
    data: &[u8],
    signature: &Signature,
    verifying_key: &VerifyingKey,
) -> Result<(), CborError> {
    // Create keccak256 digest of the data
    let digest = Keccak256::new_with_prefix(data);

    // Verify using DigestVerifier trait (type-safe, not hazmat)
    verifying_key
        .verify_digest(digest, signature)
        .map_err(|_| CborError::SignatureVerification("Signature verification failed".to_string()))
}

/// Parse a 64-byte signature (r || s) into a `Signature`.
pub fn signature_from_bytes(bytes: &[u8; 64]) -> Result<Signature, CborError> {
    Signature::from_slice(bytes)
        .map_err(|e| CborError::SignatureVerification(format!("Invalid signature: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::{
        primitives::keccak256,
        signers::{local::PrivateKeySigner, SignerSync},
    };

    /// Test private key (well-known test key, do not use in production)
    const TEST_PRIVATE_KEY: &str =
        "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

    fn test_signer() -> PrivateKeySigner {
        TEST_PRIVATE_KEY.parse().unwrap()
    }

    fn signer_verifying_key(signer: &PrivateKeySigner) -> VerifyingKey {
        verifying_key_from_bytes(&signer.public_key().0).unwrap()
    }

    // =========================================================================
    // verifying_key_from_bytes / verifying_key_to_bytes tests
    // =========================================================================

    #[test]
    fn test_verifying_key_roundtrip() {
        let signer = test_signer();
        let original_bytes = signer.public_key().0;

        let key = verifying_key_from_bytes(&original_bytes).unwrap();
        let roundtrip_bytes = verifying_key_to_bytes(&key);

        assert_eq!(original_bytes, roundtrip_bytes);
    }

    #[test]
    fn test_verifying_key_from_invalid_bytes() {
        let invalid = [0u8; 64];
        let result = verifying_key_from_bytes(&invalid);
        assert!(result.is_err());
    }

    // =========================================================================
    // verify_secp256k1 tests
    // =========================================================================

    /// Helper to convert alloy signature to k256 Signature
    fn alloy_sig_to_k256(sig: &alloy::signers::Signature) -> Signature {
        let mut bytes = [0u8; 64];
        bytes[..32].copy_from_slice(&sig.r().to_be_bytes::<32>());
        bytes[32..].copy_from_slice(&sig.s().to_be_bytes::<32>());
        signature_from_bytes(&bytes).unwrap()
    }

    #[test]
    fn test_verify_valid_signature() {
        let signer = test_signer();
        let key = signer_verifying_key(&signer);

        let data = b"test message to sign";

        // Sign: hash with keccak256, then sign the hash
        let hash = keccak256(data);
        let alloy_sig = signer
            .sign_hash_sync(&hash)
            .expect("signing should succeed");

        let signature = alloy_sig_to_k256(&alloy_sig);

        // Verify using our function (which also hashes with keccak256 internally)
        let result = verify_secp256k1(data, &signature, &key);
        assert!(result.is_ok(), "Valid signature should verify: {result:?}");
    }

    #[test]
    fn test_verify_wrong_data_fails() {
        let signer = test_signer();
        let key = signer_verifying_key(&signer);

        let data = b"original message";
        let wrong_data = b"different message";

        // Sign the original data
        let hash = keccak256(data);
        let alloy_sig = signer.sign_hash_sync(&hash).unwrap();
        let signature = alloy_sig_to_k256(&alloy_sig);

        // Verify against wrong data should fail
        let result = verify_secp256k1(wrong_data, &signature, &key);
        assert!(result.is_err(), "Wrong data should fail verification");
    }

    #[test]
    fn test_verify_wrong_key_fails() {
        let signer = test_signer();
        let key = signer_verifying_key(&signer);

        // Create a different signer
        let other_signer: PrivateKeySigner =
            "0x1111111111111111111111111111111111111111111111111111111111111111"
                .parse()
                .unwrap();
        let other_key = signer_verifying_key(&other_signer);

        let data = b"test message";
        let hash = keccak256(data);
        let alloy_sig = signer.sign_hash_sync(&hash).unwrap();
        let signature = alloy_sig_to_k256(&alloy_sig);

        // Verify with wrong key should fail
        let result = verify_secp256k1(data, &signature, &other_key);
        assert!(result.is_err(), "Wrong key should fail verification");

        // But correct key should work
        let result = verify_secp256k1(data, &signature, &key);
        assert!(result.is_ok());
    }

    #[test]
    fn test_signature_from_bytes() {
        let signer = test_signer();
        let hash = keccak256(b"test");
        let alloy_sig = signer.sign_hash_sync(&hash).unwrap();

        let mut bytes = [0u8; 64];
        bytes[..32].copy_from_slice(&alloy_sig.r().to_be_bytes::<32>());
        bytes[32..].copy_from_slice(&alloy_sig.s().to_be_bytes::<32>());

        let sig = signature_from_bytes(&bytes).unwrap();
        assert_eq!(&*sig.to_bytes(), &bytes);
    }
}
