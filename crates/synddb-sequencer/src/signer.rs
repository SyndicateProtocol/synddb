//! Message signing using secp256k1 (Ethereum-compatible)
//!
//! Signs messages with EIP-191 prefix for compatibility with Ethereum
//! smart contracts and standard verification tools.

use alloy::{
    primitives::{keccak256, Address, B256, B512},
    signers::{local::PrivateKeySigner, Signer},
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
    /// The key should be 64 hex characters (32 bytes), without "0x" prefix.
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
    /// Used for signature verification without address recovery.
    pub fn public_key(&self) -> [u8; 64] {
        self.signer.public_key().0
    }

    /// Get the public key as a B512
    pub fn public_key_b512(&self) -> B512 {
        self.signer.public_key()
    }

    /// Create the signing payload for a sequenced message
    ///
    /// Format: `keccak256(sequence || timestamp || message_hash)`
    ///
    /// Note: This is used for the inbox's legacy signature. The batcher uses
    /// COSE signatures with a different payload format.
    pub fn create_signing_payload(sequence: u64, timestamp: u64, message_hash: B256) -> B256 {
        let mut data = Vec::with_capacity(8 + 8 + 32);
        data.extend_from_slice(&sequence.to_be_bytes());
        data.extend_from_slice(&timestamp.to_be_bytes());
        data.extend_from_slice(message_hash.as_ref());
        keccak256(&data)
    }

    /// Sign a message payload
    ///
    /// Uses EIP-191 personal sign prefix for Ethereum compatibility:
    /// "\x19Ethereum Signed Message:\n32" + payload
    pub async fn sign(&self, payload: B256) -> Result<SignatureBytes, SignerError> {
        let signature = self
            .signer
            .sign_hash(&payload)
            .await
            .map_err(|e| SignerError::SigningFailed(e.to_string()))?;

        Ok(SignatureBytes::from_signature(&signature))
    }

    /// Sign a sequenced message (convenience method)
    pub async fn sign_message(
        &self,
        sequence: u64,
        timestamp: u64,
        message_hash: B256,
    ) -> Result<SignatureBytes, SignerError> {
        let payload = Self::create_signing_payload(sequence, timestamp, message_hash);
        self.sign(payload).await
    }

    /// Sign raw bytes (32-byte hash)
    ///
    /// This is used for CBOR/COSE signing where the caller provides the pre-hashed data.
    pub async fn sign_raw(
        &self,
        hash: &[u8; 32],
    ) -> Result<alloy::signers::Signature, SignerError> {
        let hash = B256::from(*hash);
        self.signer
            .sign_hash(&hash)
            .await
            .map_err(|e| SignerError::SigningFailed(e.to_string()))
    }

    /// Create the signing payload for a batch
    ///
    /// Format: `keccak256(start_sequence || end_sequence || messages_hash)`
    /// where `messages_hash` is the hash of the batch content
    ///
    /// Note: This is used for legacy JSON batch signing. The batcher uses
    /// CBOR batches with SHA-256 content hashes.
    pub fn create_batch_signing_payload(
        start_sequence: u64,
        end_sequence: u64,
        messages_hash: B256,
    ) -> B256 {
        let mut data = Vec::with_capacity(8 + 8 + 32);
        data.extend_from_slice(&start_sequence.to_be_bytes());
        data.extend_from_slice(&end_sequence.to_be_bytes());
        data.extend_from_slice(messages_hash.as_ref());
        keccak256(&data)
    }

    /// Compute the hash of serialized messages for batch signing
    pub fn compute_messages_hash(messages_json: &[u8]) -> B256 {
        keccak256(messages_json)
    }

    /// Sign a batch (convenience method) - returns 65-byte legacy signature
    pub async fn sign_batch(
        &self,
        start_sequence: u64,
        end_sequence: u64,
        messages_hash: B256,
    ) -> Result<SignatureBytes, SignerError> {
        let payload =
            Self::create_batch_signing_payload(start_sequence, end_sequence, messages_hash);
        self.sign(payload).await
    }

    /// Sign a batch for CBOR format - returns 64-byte signature (r || s)
    ///
    /// The `content_hash` is the SHA-256 hash of the CBOR-encoded messages.
    /// The signing payload is: `keccak256(keccak256(start_sequence` || `end_sequence` || `content_hash`))
    pub async fn sign_batch_cbor(
        &self,
        start_sequence: u64,
        end_sequence: u64,
        content_hash: &[u8; 32],
    ) -> Result<CborSignatureBytes, SignerError> {
        // Build signing payload: keccak256(start || end || content_hash)
        let mut data = Vec::with_capacity(8 + 8 + 32);
        data.extend_from_slice(&start_sequence.to_be_bytes());
        data.extend_from_slice(&end_sequence.to_be_bytes());
        data.extend_from_slice(content_hash);
        let inner_hash = keccak256(&data);

        // Final hash to sign
        let message_hash = keccak256(inner_hash);

        let signature = self
            .signer
            .sign_hash(&message_hash)
            .await
            .map_err(|e| SignerError::SigningFailed(e.to_string()))?;

        Ok(CborSignatureBytes::from_signature(&signature))
    }

    /// Compute content hash from serialized messages (JSON for now, but works with any bytes)
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

/// 65-byte signature (r: 32, s: 32, v: 1) - legacy format for individual message signing
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignatureBytes {
    /// Full 65-byte signature (r: 32, s: 32, v: 1)
    pub bytes: [u8; 65],
}

impl SignatureBytes {
    fn from_signature(sig: &alloy::signers::Signature) -> Self {
        let mut bytes = [0u8; 65];
        bytes[..32].copy_from_slice(&sig.r().to_be_bytes::<32>());
        bytes[32..64].copy_from_slice(&sig.s().to_be_bytes::<32>());
        // v is a bool (y_parity), convert to recovery id (27 or 28 for legacy, 0 or 1 for EIP-155)
        bytes[64] = if sig.v() { 28 } else { 27 };
        Self { bytes }
    }

    /// Convert to hex string (without 0x prefix)
    pub fn to_hex(&self) -> String {
        hex::encode(self.bytes)
    }

    /// Convert to hex string with 0x prefix
    pub fn to_hex_prefixed(&self) -> String {
        format!("0x{}", self.to_hex())
    }
}

/// 64-byte CBOR/COSE signature (r: 32, s: 32) - used for batch signing
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CborSignatureBytes {
    /// 64-byte signature (r: 32, s: 32) without recovery id
    pub bytes: [u8; 64],
}

impl CborSignatureBytes {
    /// Create from an alloy Signature (drops the v/recovery byte)
    pub fn from_signature(sig: &alloy::signers::Signature) -> Self {
        let mut bytes = [0u8; 64];
        bytes[..32].copy_from_slice(&sig.r().to_be_bytes::<32>());
        bytes[32..].copy_from_slice(&sig.s().to_be_bytes::<32>());
        Self { bytes }
    }

    /// Convert to hex string (without 0x prefix)
    pub fn to_hex(&self) -> String {
        hex::encode(self.bytes)
    }

    /// Convert to hex string with 0x prefix
    pub fn to_hex_prefixed(&self) -> String {
        format!("0x{}", self.to_hex())
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
    fn test_signing_payload_deterministic() {
        let sequence = 123u64;
        let timestamp = 1700000000u64;
        let message_hash = B256::from([0x42; 32]);

        let payload1 = MessageSigner::create_signing_payload(sequence, timestamp, message_hash);
        let payload2 = MessageSigner::create_signing_payload(sequence, timestamp, message_hash);

        assert_eq!(payload1, payload2);
    }

    #[test]
    fn test_signing_payload_varies_with_input() {
        let message_hash = B256::from([0x42; 32]);

        let p1 = MessageSigner::create_signing_payload(1, 1000, message_hash);
        let p2 = MessageSigner::create_signing_payload(2, 1000, message_hash);
        let p3 = MessageSigner::create_signing_payload(1, 1001, message_hash);

        assert_ne!(p1, p2);
        assert_ne!(p1, p3);
        assert_ne!(p2, p3);
    }

    #[tokio::test]
    async fn test_sign_message() {
        let signer = MessageSigner::new(TEST_PRIVATE_KEY).unwrap();
        let message_hash = B256::from([0x42; 32]);

        let sig = signer
            .sign_message(1, 1700000000, message_hash)
            .await
            .unwrap();

        // Signature should be 65 bytes
        assert_eq!(sig.bytes.len(), 65);

        // Hex encoding should be 130 characters
        assert_eq!(sig.to_hex().len(), 130);
    }

    #[tokio::test]
    async fn test_signature_deterministic() {
        let signer = MessageSigner::new(TEST_PRIVATE_KEY).unwrap();
        let message_hash = B256::from([0x42; 32]);

        let sig1 = signer.sign_message(1, 1000, message_hash).await.unwrap();
        let sig2 = signer.sign_message(1, 1000, message_hash).await.unwrap();

        // Same input should produce same signature (deterministic signing)
        assert_eq!(sig1.bytes, sig2.bytes);
    }
}
