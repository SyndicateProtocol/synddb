//! Signature verification for sequencer messages
//!
//! Verifies that `SignedMessage` payloads were signed by the expected sequencer.
//! All messages use `COSE_Sign1` signatures with 64-byte (r || s) format.

use crate::error::ValidatorError;
use anyhow::Result;
use synddb_shared::types::message::SignedMessage;

/// Verifies signatures on `SignedMessage` payloads
#[derive(Debug, Clone)]
pub struct SignatureVerifier {
    /// Expected sequencer public key (64 bytes, uncompressed without 0x04 prefix)
    expected_pubkey: [u8; 64],
}

impl SignatureVerifier {
    /// Create a new verifier expecting messages from the given sequencer public key
    pub const fn new(expected_pubkey: [u8; 64]) -> Self {
        Self { expected_pubkey }
    }

    /// Create a new verifier from a hex-encoded public key string
    ///
    /// The string should be 128 hex characters (with optional "0x" prefix)
    /// representing the 64-byte uncompressed public key.
    pub fn from_hex(hex_pubkey: &str) -> Result<Self> {
        let hex_str = hex_pubkey.strip_prefix("0x").unwrap_or(hex_pubkey);
        if hex_str.len() != 128 {
            return Err(ValidatorError::InvalidSignature(format!(
                "Invalid public key length: expected 128 hex chars, got {}",
                hex_str.len()
            ))
            .into());
        }
        let bytes = hex::decode(hex_str).map_err(|e| {
            ValidatorError::InvalidSignature(format!("Invalid public key hex: {e}"))
        })?;
        let mut pubkey = [0u8; 64];
        pubkey.copy_from_slice(&bytes);
        Ok(Self::new(pubkey))
    }

    /// Verify a signed message
    ///
    /// All messages use `COSE_Sign1` signatures. This method:
    /// 1. Verifies the COSE signature is valid
    /// 2. Verifies the claimed signer matches expected sequencer public key
    pub fn verify(&self, message: &SignedMessage) -> Result<()> {
        // Verify signature using COSE format
        message.verify_signature().map_err(|e| {
            ValidatorError::SignatureVerification(format!("Signature verification failed: {e}"))
        })?;

        // Verify the signer matches our expected sequencer public key
        let hex_str = message.signer.strip_prefix("0x").unwrap_or(&message.signer);
        if hex_str.len() != 128 {
            return Err(ValidatorError::InvalidSignature(format!(
                "Invalid signer public key length: expected 128 hex chars, got {}",
                hex_str.len()
            ))
            .into());
        }
        let claimed_pubkey = hex::decode(hex_str).map_err(|e| {
            ValidatorError::InvalidSignature(format!("Invalid signer public key hex: {e}"))
        })?;

        if claimed_pubkey != self.expected_pubkey {
            return Err(ValidatorError::SignerMismatch {
                expected: format!("0x{}", hex::encode(self.expected_pubkey)),
                actual: format!("0x{}", hex::encode(&claimed_pubkey)),
            }
            .into());
        }

        Ok(())
    }

    /// Get the expected public key as hex string
    pub fn expected_pubkey_hex(&self) -> String {
        format!("0x{}", hex::encode(self.expected_pubkey))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::{
        primitives::{keccak256, B256},
        signers::{local::PrivateKeySigner, SignerSync},
    };
    use synddb_shared::types::cbor::{
        error::CborError,
        message::{CborMessageType, CborSignedMessage},
        verify::verifying_key_from_bytes,
    };

    // Test private key (DO NOT use in production!)
    // Address: 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266
    const TEST_PRIVATE_KEY: &str =
        "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

    /// Sign a message synchronously (returns 64-byte signature for COSE)
    fn sign_cose(signer: &PrivateKeySigner, data: &[u8]) -> Result<[u8; 64], CborError> {
        let hash = keccak256(data);
        let sig = signer
            .sign_hash_sync(&B256::from(hash))
            .map_err(|e| CborError::Signing(e.to_string()))?;

        // Extract r and s (64 bytes total, no v)
        let mut result = [0u8; 64];
        result[..32].copy_from_slice(&sig.r().to_be_bytes::<32>());
        result[32..].copy_from_slice(&sig.s().to_be_bytes::<32>());
        Ok(result)
    }

    /// Get signer's 64-byte uncompressed public key (without 0x04 prefix)
    fn signer_pubkey(signer: &PrivateKeySigner) -> [u8; 64] {
        let pubkey = signer.credential().verifying_key().to_encoded_point(false);
        let bytes = pubkey.as_bytes();
        let mut result = [0u8; 64];
        result.copy_from_slice(&bytes[1..65]);
        result
    }

    /// Create a test message in COSE format
    fn create_test_message(sequence: u64, payload: &[u8]) -> SignedMessage {
        let signer: PrivateKeySigner = TEST_PRIVATE_KEY.parse().unwrap();
        let pubkey_bytes = signer_pubkey(&signer);
        let pubkey = verifying_key_from_bytes(&pubkey_bytes).unwrap();
        let timestamp = 1700000000 + sequence;

        // Create COSE-signed message
        let cbor_msg = CborSignedMessage::new(
            sequence,
            timestamp,
            CborMessageType::Changeset,
            payload.to_vec(),
            &pubkey,
            |data| sign_cose(&signer, data),
        )
        .unwrap();

        // Convert to SignedMessage format
        cbor_msg.to_signed_message(&pubkey).unwrap()
    }

    #[test]
    fn test_verify_valid_signature() {
        let signer: PrivateKeySigner = TEST_PRIVATE_KEY.parse().unwrap();
        let pubkey = signer_pubkey(&signer);
        let verifier = SignatureVerifier::new(pubkey);

        let message = create_test_message(0, b"test payload");

        // Verify COSE message has protected header
        assert!(
            !message.cose_protected_header.is_empty(),
            "COSE message should have protected header"
        );

        // Verify signature is 64 bytes (128 hex chars + "0x")
        let sig_hex_len = message.signature.len();
        assert_eq!(
            sig_hex_len,
            2 + 128,
            "COSE signature should be 64 bytes (got {} hex chars)",
            sig_hex_len - 2
        );

        // Verification should succeed
        assert!(verifier.verify(&message).is_ok());
    }

    #[test]
    fn test_verify_wrong_signer_fails() {
        // Create verifier expecting a different public key
        let wrong_pubkey = [0x01u8; 64]; // All 0x01s
        let verifier = SignatureVerifier::new(wrong_pubkey);

        let message = create_test_message(0, b"test payload");
        let result = verifier.verify(&message);

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("Signer mismatch"),
            "Expected signer mismatch error, got: {err}"
        );
    }

    #[test]
    fn test_verify_tampered_payload_fails() {
        let signer: PrivateKeySigner = TEST_PRIVATE_KEY.parse().unwrap();
        let pubkey = signer_pubkey(&signer);
        let verifier = SignatureVerifier::new(pubkey);

        let mut message = create_test_message(0, b"original payload");

        // Tamper with the payload
        message.payload = b"tampered payload".to_vec();

        let result = verifier.verify(&message);
        assert!(result.is_err());
        // COSE verification fails because payload in Sig_structure won't match
    }

    #[test]
    fn test_verify_tampered_sequence_fails() {
        // Tampering with the outer sequence field fails because it must match protected header
        let signer: PrivateKeySigner = TEST_PRIVATE_KEY.parse().unwrap();
        let pubkey = signer_pubkey(&signer);
        let verifier = SignatureVerifier::new(pubkey);

        let mut message = create_test_message(0, b"test payload");
        message.sequence = 999;

        let result = verifier.verify(&message);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("Header mismatch") && err.contains("sequence"),
            "Expected header mismatch error for sequence, got: {err}"
        );
    }

    #[test]
    fn test_verify_tampered_timestamp_fails() {
        let signer: PrivateKeySigner = TEST_PRIVATE_KEY.parse().unwrap();
        let pubkey = signer_pubkey(&signer);
        let verifier = SignatureVerifier::new(pubkey);

        let mut message = create_test_message(0, b"test payload");
        message.timestamp = 9999999999;

        let result = verifier.verify(&message);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("Header mismatch") && err.contains("timestamp"),
            "Expected header mismatch error for timestamp, got: {err}"
        );
    }

    #[test]
    fn test_verify_invalid_signature_format() {
        let signer: PrivateKeySigner = TEST_PRIVATE_KEY.parse().unwrap();
        let pubkey = signer_pubkey(&signer);
        let verifier = SignatureVerifier::new(pubkey);

        let mut message = create_test_message(0, b"test payload");
        message.signature = "0xdeadbeef".to_string();

        let result = verifier.verify(&message);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.to_lowercase().contains("signature"),
            "Expected signature error, got: {err}"
        );
    }

    #[test]
    fn test_verify_tampered_protected_header_fails() {
        let signer: PrivateKeySigner = TEST_PRIVATE_KEY.parse().unwrap();
        let pubkey = signer_pubkey(&signer);
        let verifier = SignatureVerifier::new(pubkey);

        let mut message = create_test_message(0, b"test payload");

        // Tamper with the protected header
        if !message.cose_protected_header.is_empty() {
            message.cose_protected_header[0] ^= 0xFF;
        }

        let result = verifier.verify(&message);
        assert!(result.is_err());
        // Verification fails because the Sig_structure won't match
    }
}
