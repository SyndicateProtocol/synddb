//! Signature verification for sequencer messages
//!
//! Verifies that `SignedMessage` payloads were signed by the expected sequencer.
//!
//! Supports both signature formats:
//! - **Legacy (JSON)**: 65-byte signature over `keccak256(sequence || timestamp || message_hash)`
//! - **COSE**: 64-byte signature over COSE `Sig_structure` (detected via `cose_protected_header`)

use crate::error::ValidatorError;
use alloy::primitives::{keccak256, Address};
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
    /// Automatically handles both legacy (JSON) and COSE signature formats.
    ///
    /// Checks:
    /// 1. Payload hash matches `message_hash` (for legacy format only)
    /// 2. Signature is valid (delegates to `SignedMessage::verify_signature()`)
    /// 3. Claimed signer matches expected sequencer
    pub fn verify(&self, message: &SignedMessage) -> Result<()> {
        // For legacy format, verify payload hash matches message_hash
        // (COSE format doesn't need this check as payload is in the Sig_structure)
        if message.cose_protected_header.is_none() {
            let computed_hash = keccak256(&message.payload);
            let computed_hash_hex = format!("0x{}", hex::encode(computed_hash));

            if computed_hash_hex != message.message_hash {
                return Err(ValidatorError::PayloadHashMismatch {
                    expected: message.message_hash.clone(),
                    computed: computed_hash_hex,
                }
                .into());
            }
        }

        // Verify signature using the appropriate format (legacy or COSE)
        // This delegates to SignedMessage::verify_signature() which handles both formats
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
        primitives::B256,
        signers::{local::PrivateKeySigner, Signer},
    };
    use synddb_shared::types::message::MessageType;

    // Test private key (DO NOT use in production!)
    // Address: 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266
    const TEST_PRIVATE_KEY: &str =
        "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

    async fn create_test_message(sequence: u64, payload: &[u8]) -> SignedMessage {
        let signer: PrivateKeySigner = TEST_PRIVATE_KEY.parse().unwrap();
        let timestamp = 1700000000 + sequence;

        // Compress payload (use simple copy for tests)
        let compressed_payload = payload.to_vec();

        // Hash the payload
        let message_hash = keccak256(&compressed_payload);

        // Create signing payload using SignedMessage helper
        let signing_payload =
            SignedMessage::compute_signing_payload(sequence, timestamp, message_hash);

        // Sign
        let signature = signer.sign_hash(&signing_payload).await.unwrap();

        // Format signature as 65 bytes
        let mut sig_bytes = [0u8; 65];
        sig_bytes[..32].copy_from_slice(&signature.r().to_be_bytes::<32>());
        sig_bytes[32..64].copy_from_slice(&signature.s().to_be_bytes::<32>());
        sig_bytes[64] = if signature.v() { 28 } else { 27 };

        SignedMessage {
            sequence,
            timestamp,
            message_type: MessageType::Changeset,
            payload: compressed_payload,
            message_hash: format!("0x{}", hex::encode(message_hash)),
            signature: format!("0x{}", hex::encode(sig_bytes)),
            signer: format!("{:?}", signer.address()),
            cose_protected_header: None,
        }
    }

    #[tokio::test]
    async fn test_verify_valid_signature() {
        let signer: PrivateKeySigner = TEST_PRIVATE_KEY.parse().unwrap();
        let verifier = SignatureVerifier::new(signer.address());

        let message = create_test_message(0, b"test payload").await;
        assert!(verifier.verify(&message).is_ok());
    }

    #[tokio::test]
    async fn test_verify_wrong_signer_fails() {
        // Create verifier expecting a different address
        let wrong_address: Address = "0x0000000000000000000000000000000000000001"
            .parse()
            .unwrap();
        let verifier = SignatureVerifier::new(wrong_address);

        let message = create_test_message(0, b"test payload").await;
        let result = verifier.verify(&message);

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Signer mismatch"));
    }

    #[tokio::test]
    async fn test_verify_tampered_payload_fails() {
        let signer: PrivateKeySigner = TEST_PRIVATE_KEY.parse().unwrap();
        let verifier = SignatureVerifier::new(signer.address());

        let mut message = create_test_message(0, b"original payload").await;
        // Tamper with payload
        message.payload = b"tampered payload".to_vec();

        let result = verifier.verify(&message);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("hash mismatch"));
    }

    #[tokio::test]
    async fn test_verify_tampered_sequence_fails() {
        let signer: PrivateKeySigner = TEST_PRIVATE_KEY.parse().unwrap();
        let verifier = SignatureVerifier::new(signer.address());

        let mut message = create_test_message(0, b"test payload").await;
        // Tamper with sequence (but keep valid hash)
        message.sequence = 999;

        // Recalculate message_hash to match payload
        message.message_hash = format!("0x{}", hex::encode(keccak256(&message.payload)));

        let result = verifier.verify(&message);
        assert!(result.is_err());
        // Signature verification should fail because signing payload has wrong sequence
    }

    #[tokio::test]
    async fn test_verify_invalid_signature_format() {
        let signer: PrivateKeySigner = TEST_PRIVATE_KEY.parse().unwrap();
        let verifier = SignatureVerifier::new(signer.address());

        let mut message = create_test_message(0, b"test payload").await;
        // Invalid signature (wrong length)
        message.signature = "0xdeadbeef".to_string();

        let result = verifier.verify(&message);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("signature"));
    }

    #[test]
    fn test_signing_payload_deterministic() {
        let hash = B256::from([0x42; 32]);
        let p1 = SignedMessage::compute_signing_payload(1, 1000, hash);
        let p2 = SignedMessage::compute_signing_payload(1, 1000, hash);
        assert_eq!(p1, p2);
    }

    #[test]
    fn test_signing_payload_varies() {
        let hash = B256::from([0x42; 32]);
        let p1 = SignedMessage::compute_signing_payload(1, 1000, hash);
        let p2 = SignedMessage::compute_signing_payload(2, 1000, hash);
        let p3 = SignedMessage::compute_signing_payload(1, 1001, hash);
        assert_ne!(p1, p2);
        assert_ne!(p1, p3);
    }
}
