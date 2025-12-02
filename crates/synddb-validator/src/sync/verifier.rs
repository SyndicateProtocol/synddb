//! Signature verification for sequencer messages
//!
//! Verifies that `SignedMessage` payloads were signed by the expected sequencer.

use crate::error::ValidatorError;
use alloy::primitives::{keccak256, Address, B256};
use anyhow::{Context, Result};
use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};
use synddb_shared::types::SignedMessage;

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
    /// Checks:
    /// 1. Payload hash matches `message_hash`
    /// 2. Signature is valid over the signing payload
    /// 3. Recovered signer matches `message.signer`
    /// 4. Recovered signer matches expected sequencer
    pub fn verify(&self, message: &SignedMessage) -> Result<()> {
        // 1. Verify payload hash matches
        let computed_hash = keccak256(&message.payload);
        let computed_hash_hex = format!("0x{}", hex::encode(computed_hash));

        if computed_hash_hex != message.message_hash {
            return Err(ValidatorError::PayloadHashMismatch {
                expected: message.message_hash.clone(),
                computed: computed_hash_hex,
            }
            .into());
        }

        // 2. Reconstruct signing payload (same format as sequencer)
        // Format: keccak256(sequence || timestamp || message_hash)
        let signing_payload =
            Self::create_signing_payload(message.sequence, message.timestamp, computed_hash);

        // 3. Parse signature (65 bytes: r[32] + s[32] + v[1])
        let sig_bytes = hex::decode(
            message
                .signature
                .strip_prefix("0x")
                .unwrap_or(&message.signature),
        )
        .context("Invalid signature hex")?;

        if sig_bytes.len() != 65 {
            return Err(ValidatorError::InvalidSignature(format!(
                "Expected 65 bytes, got {}",
                sig_bytes.len()
            ))
            .into());
        }

        // 4. Recover public key
        let v = sig_bytes[64];
        let recovery_id = RecoveryId::try_from(if v >= 27 { v - 27 } else { v })
            .map_err(|e| ValidatorError::InvalidSignature(format!("Invalid recovery id: {e}")))?;

        let signature = Signature::from_slice(&sig_bytes[0..64])
            .map_err(|e| ValidatorError::InvalidSignature(format!("Invalid signature: {e}")))?;

        let recovered_key =
            VerifyingKey::recover_from_prehash(signing_payload.as_slice(), &signature, recovery_id)
                .map_err(|e| {
                    ValidatorError::SignatureVerification(format!(
                        "Failed to recover public key: {e}"
                    ))
                })?;

        // 5. Derive Ethereum address from public key
        let public_key_bytes = recovered_key.to_encoded_point(false);
        let pk_hash = keccak256(&public_key_bytes.as_bytes()[1..]);
        let recovered_address = Address::from_slice(&pk_hash[12..]);

        // 6. Verify against message.signer
        let claimed_signer: Address = message
            .signer
            .parse()
            .context("Invalid signer address in message")?;

        if recovered_address != claimed_signer {
            return Err(ValidatorError::SignerMismatch {
                expected: format!("{claimed_signer:?}"),
                actual: format!("{recovered_address:?}"),
            }
            .into());
        }

        // 7. Verify against expected sequencer
        if recovered_address != self.expected_signer {
            return Err(ValidatorError::SignerMismatch {
                expected: format!("{:?}", self.expected_signer),
                actual: format!("{recovered_address:?}"),
            }
            .into());
        }

        Ok(())
    }

    /// Create the signing payload (must match sequencer's format exactly)
    ///
    /// Format: `keccak256(sequence_be || timestamp_be || message_hash)`
    fn create_signing_payload(sequence: u64, timestamp: u64, message_hash: B256) -> B256 {
        let mut data = Vec::with_capacity(48);
        data.extend_from_slice(&sequence.to_be_bytes());
        data.extend_from_slice(&timestamp.to_be_bytes());
        data.extend_from_slice(message_hash.as_slice());
        keccak256(&data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::signers::local::PrivateKeySigner;
    use alloy::signers::Signer;
    use synddb_shared::types::MessageType;

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

        // Create signing payload
        let signing_payload =
            SignatureVerifier::create_signing_payload(sequence, timestamp, message_hash);

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
        assert!(err.contains("Expected 65 bytes"));
    }

    #[test]
    fn test_signing_payload_deterministic() {
        let hash = B256::from([0x42; 32]);
        let p1 = SignatureVerifier::create_signing_payload(1, 1000, hash);
        let p2 = SignatureVerifier::create_signing_payload(1, 1000, hash);
        assert_eq!(p1, p2);
    }

    #[test]
    fn test_signing_payload_varies() {
        let hash = B256::from([0x42; 32]);
        let p1 = SignatureVerifier::create_signing_payload(1, 1000, hash);
        let p2 = SignatureVerifier::create_signing_payload(2, 1000, hash);
        let p3 = SignatureVerifier::create_signing_payload(1, 1001, hash);
        assert_ne!(p1, p2);
        assert_ne!(p1, p3);
    }
}
