//! Signature verification for sequencer messages
//!
//! Verifies that `SignedMessage` payloads were signed by the expected sequencer.
//! All messages use `COSE_Sign1` signatures with 64-byte (r || s) format.

use crate::error::ValidatorError;
use alloy::primitives::Address;
use anyhow::Result;
use synddb_shared::types::message::SignedMessage;

/// Verifies signatures on `SignedMessage` payloads
#[derive(Debug, Clone)]
pub struct SignatureVerifier {
    /// Expected sequencer address
    expected_signer: Address,
}

impl SignatureVerifier {
    /// Create a new verifier expecting messages from the given sequencer
    pub const fn new(expected_signer: Address) -> Self {
        Self { expected_signer }
    }

    /// Verify a signed message
    ///
    /// All messages use `COSE_Sign1` signatures. This method:
    /// 1. Verifies the COSE signature is valid
    /// 2. Verifies the claimed signer matches expected sequencer
    pub fn verify(&self, message: &SignedMessage) -> Result<()> {
        // Verify signature using COSE format
        message.verify_signature().map_err(|e| {
            ValidatorError::SignatureVerification(format!("Signature verification failed: {e}"))
        })?;

        // Verify the signer matches our expected sequencer
        let claimed_signer: Address = message.signer.parse().map_err(|e| {
            ValidatorError::InvalidSignature(format!("Invalid signer address: {e}"))
        })?;

        if claimed_signer != self.expected_signer {
            return Err(ValidatorError::SignerMismatch {
                expected: format!("{:?}", self.expected_signer),
                actual: format!("{claimed_signer:?}"),
            }
            .into());
        }

        Ok(())
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

    /// Create a test message in COSE format
    fn create_test_message(sequence: u64, payload: &[u8]) -> SignedMessage {
        let signer: PrivateKeySigner = TEST_PRIVATE_KEY.parse().unwrap();
        let addr = signer.address().into_array();
        let timestamp = 1700000000 + sequence;

        // Create COSE-signed message
        let cbor_msg = CborSignedMessage::new(
            sequence,
            timestamp,
            CborMessageType::Changeset,
            payload.to_vec(),
            addr,
            |data| sign_cose(&signer, data),
        )
        .unwrap();

        // Convert to SignedMessage format
        cbor_msg.to_signed_message(&addr).unwrap()
    }

    #[test]
    fn test_verify_valid_signature() {
        let signer: PrivateKeySigner = TEST_PRIVATE_KEY.parse().unwrap();
        let verifier = SignatureVerifier::new(signer.address());

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
        // Create verifier expecting a different address
        let wrong_address: Address = "0x0000000000000000000000000000000000000001"
            .parse()
            .unwrap();
        let verifier = SignatureVerifier::new(wrong_address);

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
        let verifier = SignatureVerifier::new(signer.address());

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
        let verifier = SignatureVerifier::new(signer.address());

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
        let verifier = SignatureVerifier::new(signer.address());

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
        let verifier = SignatureVerifier::new(signer.address());

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
        let verifier = SignatureVerifier::new(signer.address());

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
