//! Message inbox for ordering and sequencing
//!
//! Provides monotonic sequence number assignment for all incoming messages.
//! This is the core ordering mechanism similar to Arbitrum's delayed inbox.
//!
//! Messages are signed using COSE (CBOR Object Signing and Encryption) with
//! 64-byte secp256k1 ECDSA signatures (r || s format, no recovery ID).
//!
//! For withdrawals, a Bridge-compatible signature is also generated using
//! EIP-191 personal sign format, matching what the Bridge contract expects.

use alloy::primitives::{keccak256, Address, B256, U256};
use k256::ecdsa::Signature;
use std::{
    io::Write,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{SystemTime, UNIX_EPOCH},
};
use synddb_shared::{
    keys::EvmKeyManager,
    types::cbor::{
        error::CborError,
        message::{CborMessageType, CborSignedMessage},
        verify::{signature_from_bytes, verifying_key_from_bytes},
    },
};
use thiserror::Error;
use tracing::debug;

/// Errors from inbox operations
#[derive(Debug, Error)]
pub enum InboxError {
    #[error("Signing error: {0}")]
    Signing(String),

    #[error("CBOR encoding error: {0}")]
    Cbor(#[from] CborError),

    #[error("Invalid public key: {0}")]
    InvalidPublicKey(String),
}

/// Receipt returned to clients after successful sequencing
///
/// Contains the COSE signature (64 bytes, r || s format) for the sequenced message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SequenceReceipt {
    /// Assigned sequence number
    pub sequence: u64,
    /// Timestamp when sequenced
    pub timestamp: u64,
    /// Hash of the compressed payload: `keccak256(zstd_compressed_payload)`
    pub message_hash: String,
    /// COSE signature (64 bytes, r || s). Encoded as hex with 0x prefix.
    pub signature: String,
    /// 64-byte uncompressed public key (without 0x04 prefix). Encoded as hex with 0x prefix.
    pub signer: String,
}

/// Bridge-compatible signature for withdrawals
///
/// This signature matches the format expected by the Bridge contract's
/// `initializeMessage` function. It uses EIP-191 personal sign format over
/// the Bridge message hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeSignature {
    /// The message ID (`request_id` as bytes32)
    pub message_id: String,
    /// The Bridge message hash that was signed
    pub message_hash: String,
    /// EIP-191 signature (65 bytes: r || s || v). Encoded as hex with 0x prefix.
    pub signature: String,
    /// Signer address (20 bytes). Encoded as hex with 0x prefix.
    pub signer: String,
}

/// Compress payload using `zstd`
///
/// Uses compression level 3 (default) for a good balance of speed and compression ratio.
fn compress_payload(data: &[u8]) -> Vec<u8> {
    let mut encoder = zstd::Encoder::new(Vec::new(), 3).expect("Failed to create zstd encoder");
    encoder
        .write_all(data)
        .expect("Failed to write data to encoder");
    encoder.finish().expect("Failed to finish compression")
}

/// The message inbox - assigns sequence numbers and signs messages using COSE
#[derive(Debug)]
pub struct Inbox {
    /// Current sequence counter (atomic for thread safety)
    sequence: AtomicU64,
    /// TEE key manager for signing
    key_manager: Arc<EvmKeyManager>,
}

impl Inbox {
    /// Create a new inbox with a shared key manager, starting from sequence 0
    pub const fn new(key_manager: Arc<EvmKeyManager>) -> Self {
        Self::with_start_sequence(key_manager, 0)
    }

    /// Create a new inbox with a shared key manager, starting from a specific sequence
    ///
    /// Use this when the key manager is shared with other components (e.g., batcher).
    pub const fn with_start_sequence(key_manager: Arc<EvmKeyManager>, start_sequence: u64) -> Self {
        Self {
            sequence: AtomicU64::new(start_sequence),
            key_manager,
        }
    }

    /// Get the signer's address
    pub fn signer_address(&self) -> Address {
        self.key_manager.address()
    }

    /// Get a reference to the key manager (for sharing with batcher)
    pub fn key_manager(&self) -> Arc<EvmKeyManager> {
        Arc::clone(&self.key_manager)
    }

    /// Get the current sequence number (next to be assigned)
    pub fn current_sequence(&self) -> u64 {
        self.sequence.load(Ordering::SeqCst)
    }

    /// Sequence and sign a message using COSE
    ///
    /// This atomically:
    /// 1. Assigns the next sequence number
    /// 2. Records the current timestamp
    /// 3. Compresses the payload with `zstd`
    /// 4. Creates a COSE `Sign1` message with 64-byte signature
    /// 5. Returns the signed message and a receipt
    pub fn sequence_message(
        &self,
        message_type: CborMessageType,
        payload: Vec<u8>,
    ) -> Result<(CborSignedMessage, SequenceReceipt), InboxError> {
        let start = std::time::Instant::now();

        // Atomically get and increment sequence
        let sequence = self.sequence.fetch_add(1, Ordering::SeqCst);

        // Get current timestamp
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards")
            .as_secs();

        // Compress the payload
        let compressed_payload = compress_payload(&payload);

        // Hash the compressed payload (for receipt)
        let message_hash = keccak256(&compressed_payload);

        // Get signer's public key as `VerifyingKey`
        let pubkey_bytes = self.key_manager.public_key();
        let verifying_key = verifying_key_from_bytes(&pubkey_bytes)
            .map_err(|e| InboxError::InvalidPublicKey(e.to_string()))?;

        // Create COSE signed message
        let cbor_message = CborSignedMessage::new(
            sequence,
            timestamp,
            message_type,
            compressed_payload,
            &verifying_key,
            |data| self.sign_cose(data),
        )?;

        // Extract signature from COSE message for receipt
        let parsed = cbor_message.parse_without_verify()?;
        let signature_hex = format!("0x{}", hex::encode(parsed.signature));
        let signer_hex = format!("0x{}", hex::encode(pubkey_bytes));
        let message_hash_hex = format!("0x{}", hex::encode(message_hash));

        let receipt = SequenceReceipt {
            sequence,
            timestamp,
            message_hash: message_hash_hex,
            signature: signature_hex,
            signer: signer_hex,
        };

        // Record metrics
        let message_type_str = match message_type {
            CborMessageType::Changeset => "changeset",
            CborMessageType::Withdrawal => "withdrawal",
            CborMessageType::Snapshot => "snapshot",
        };
        crate::metrics::record_message_sequenced(
            message_type_str,
            cbor_message.size(),
            start.elapsed().as_secs_f64(),
        );
        crate::metrics::update_current_sequence(sequence + 1);

        debug!(
            sequence,
            message_type = message_type_str,
            payload_size = payload.len(),
            compressed_size = cbor_message.size(),
            elapsed_us = start.elapsed().as_micros(),
            "Message sequenced"
        );

        Ok((cbor_message, receipt))
    }

    /// Sign data for COSE (returns ECDSA Signature)
    fn sign_cose(&self, data: &[u8]) -> Result<Signature, CborError> {
        let hash = keccak256(data);
        let alloy_sig = self
            .key_manager
            .sign_raw_sync(&hash.0)
            .map_err(|e| CborError::Signing(e.to_string()))?;

        let mut bytes = [0u8; 64];
        bytes[..32].copy_from_slice(&alloy_sig.r().to_be_bytes::<32>());
        bytes[32..].copy_from_slice(&alloy_sig.s().to_be_bytes::<32>());
        signature_from_bytes(&bytes)
    }

    /// Sign a withdrawal for the Bridge contract
    ///
    /// Creates a signature compatible with the Bridge contract's `initializeMessage` function.
    /// The signature is over the EIP-191 personal sign hash of the Bridge message hash.
    ///
    /// Bridge message hash format:
    /// ```solidity
    /// keccak256(abi.encodePacked(messageId, targetAddress, keccak256(payload), nativeTokenAmount))
    /// ```
    pub fn sign_bridge_withdrawal(
        &self,
        message_id: &str,
        target_address: &str,
        payload: &[u8],
        native_token_amount: &str,
    ) -> Result<BridgeSignature, InboxError> {
        // Parse message_id to bytes32
        // If it looks like a hex string (0x prefix or 64 hex chars), decode it
        // Otherwise, hash the string to get a bytes32
        let message_id_b256 = {
            let message_id_hex = message_id.strip_prefix("0x").unwrap_or(message_id);
            if message_id_hex.len() == 64 && message_id_hex.chars().all(|c| c.is_ascii_hexdigit()) {
                // Valid 32-byte hex string
                let bytes = hex::decode(message_id_hex)
                    .map_err(|e| InboxError::Signing(format!("Invalid message_id hex: {e}")))?;
                B256::from_slice(&bytes)
            } else if message_id_hex.chars().all(|c| c.is_ascii_hexdigit())
                && !message_id_hex.is_empty()
            {
                // Shorter hex string - pad left with zeros
                let bytes = hex::decode(message_id_hex)
                    .map_err(|e| InboxError::Signing(format!("Invalid message_id hex: {e}")))?;
                let mut padded = [0u8; 32];
                let start = 32 - bytes.len().min(32);
                padded[start..].copy_from_slice(&bytes[..bytes.len().min(32)]);
                B256::from(padded)
            } else {
                // Non-hex string - hash it to get a bytes32
                keccak256(message_id.as_bytes())
            }
        };

        // Parse target address
        let target_hex = target_address.strip_prefix("0x").unwrap_or(target_address);
        let target_bytes = hex::decode(target_hex)
            .map_err(|e| InboxError::Signing(format!("Invalid target address hex: {e}")))?;
        if target_bytes.len() != 20 {
            return Err(InboxError::Signing(format!(
                "Invalid target address length: expected 20 bytes, got {}",
                target_bytes.len()
            )));
        }
        let target_address_bytes: [u8; 20] = target_bytes
            .try_into()
            .map_err(|_| InboxError::Signing("Invalid target address".to_string()))?;

        // Parse native token amount
        let amount = native_token_amount
            .parse::<u128>()
            .map_err(|e| InboxError::Signing(format!("Invalid amount: {e}")))?;
        let amount_u256 = U256::from(amount);

        // Hash the payload
        let payload_hash = keccak256(payload);

        // Compute Bridge message hash: keccak256(abi.encodePacked(messageId, targetAddress, keccak256(payload), nativeTokenAmount))
        let mut packed = Vec::with_capacity(32 + 20 + 32 + 32);
        packed.extend_from_slice(message_id_b256.as_slice());
        packed.extend_from_slice(&target_address_bytes);
        packed.extend_from_slice(payload_hash.as_slice());
        packed.extend_from_slice(&amount_u256.to_be_bytes::<32>());
        let bridge_message_hash = keccak256(&packed);

        // Apply EIP-191 personal sign: keccak256("\x19Ethereum Signed Message:\n32" + hash)
        let mut eth_signed_data = Vec::with_capacity(60);
        eth_signed_data.extend_from_slice(b"\x19Ethereum Signed Message:\n32");
        eth_signed_data.extend_from_slice(bridge_message_hash.as_slice());
        let eth_signed_hash = keccak256(&eth_signed_data);

        // Sign the EIP-191 hash
        let alloy_sig = self
            .key_manager
            .sign_raw_sync(&eth_signed_hash.0)
            .map_err(|e| InboxError::Signing(e.to_string()))?;

        // Format as r || s || v (65 bytes)
        let mut sig_bytes = Vec::with_capacity(65);
        sig_bytes.extend_from_slice(&alloy_sig.r().to_be_bytes::<32>());
        sig_bytes.extend_from_slice(&alloy_sig.s().to_be_bytes::<32>());
        // Ethereum uses v = 27 or 28
        sig_bytes.push(if alloy_sig.v() { 28 } else { 27 });

        debug!(
            message_id = %message_id,
            target = %target_address,
            signer = %self.key_manager.address(),
            "Signed Bridge withdrawal"
        );

        Ok(BridgeSignature {
            message_id: format!("{message_id_b256:#x}"),
            message_hash: format!("{bridge_message_hash:#x}"),
            signature: format!("0x{}", hex::encode(&sig_bytes)),
            signer: format!("{:#x}", self.key_manager.address()),
        })
    }
}

/// Verify that a receipt's signer matches the expected public key
///
/// The receipt contains a 64-byte uncompressed public key (hex with 0x prefix).
pub fn verify_receipt_signer(receipt: &SequenceReceipt, expected_pubkey: &[u8; 64]) -> bool {
    let receipt_pubkey_hex = receipt.signer.strip_prefix("0x").unwrap_or(&receipt.signer);
    let Ok(receipt_pubkey_bytes) = hex::decode(receipt_pubkey_hex) else {
        return false;
    };

    receipt_pubkey_bytes.as_slice() == expected_pubkey
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_key_manager() -> Arc<EvmKeyManager> {
        Arc::new(EvmKeyManager::generate())
    }

    #[test]
    fn test_inbox_creation() {
        let inbox = Inbox::new(test_key_manager());
        assert_eq!(inbox.current_sequence(), 0);
    }

    #[test]
    fn test_inbox_with_start_sequence() {
        let inbox = Inbox::with_start_sequence(test_key_manager(), 100);
        assert_eq!(inbox.current_sequence(), 100);
    }

    #[test]
    fn test_sequence_message() {
        let key_manager = test_key_manager();
        let pubkey = key_manager.public_key();
        let inbox = Inbox::new(key_manager);
        let payload = b"test payload".to_vec();

        let (cbor_msg, receipt) = inbox
            .sequence_message(CborMessageType::Changeset, payload)
            .unwrap();

        assert_eq!(receipt.sequence, 0);
        // COSE message should have non-zero size
        assert!(cbor_msg.size() > 0);
        // Receipt should have 64-byte signature (128 hex chars + 0x prefix)
        assert!(receipt.signature.starts_with("0x"));
        assert_eq!(receipt.signature.len(), 130); // 0x + 128 hex chars
                                                  // Receipt should have 64-byte public key (128 hex chars + 0x prefix)
        assert!(receipt.signer.starts_with("0x"));
        assert_eq!(receipt.signer.len(), 130); // 0x + 128 hex chars
                                               // Verify signer matches
        assert!(verify_receipt_signer(&receipt, &pubkey));
    }

    #[test]
    fn test_sequence_monotonic() {
        let inbox = Inbox::new(test_key_manager());

        let (_, receipt1) = inbox
            .sequence_message(CborMessageType::Changeset, b"msg1".to_vec())
            .unwrap();

        let (_, receipt2) = inbox
            .sequence_message(CborMessageType::Changeset, b"msg2".to_vec())
            .unwrap();

        let (_, receipt3) = inbox
            .sequence_message(CborMessageType::Withdrawal, b"msg3".to_vec())
            .unwrap();

        assert_eq!(receipt1.sequence, 0);
        assert_eq!(receipt2.sequence, 1);
        assert_eq!(receipt3.sequence, 2);
        assert_eq!(inbox.current_sequence(), 3);
    }

    #[test]
    fn test_receipt_signature_is_valid_cose() {
        let key_manager = test_key_manager();
        let pubkey_bytes = key_manager.public_key();
        let inbox = Inbox::new(key_manager);

        let (cbor_msg, receipt) = inbox
            .sequence_message(CborMessageType::Changeset, b"test".to_vec())
            .unwrap();

        // The COSE message should be verifiable
        let verifying_key = verifying_key_from_bytes(&pubkey_bytes).unwrap();
        let parsed = cbor_msg.verify_and_parse(&verifying_key).unwrap();

        assert_eq!(parsed.sequence, receipt.sequence);
        assert_eq!(parsed.timestamp, receipt.timestamp);

        // Signature in receipt should match the COSE signature
        let receipt_sig_hex = receipt.signature.strip_prefix("0x").unwrap();
        let receipt_sig = hex::decode(receipt_sig_hex).unwrap();
        assert_eq!(receipt_sig.as_slice(), parsed.signature.as_slice());
    }

    #[test]
    fn test_verify_receipt_signer() {
        let key_manager = test_key_manager();
        let expected_pubkey = key_manager.public_key();
        let inbox = Inbox::new(key_manager);

        let (_, receipt) = inbox
            .sequence_message(CborMessageType::Changeset, b"test".to_vec())
            .unwrap();

        assert!(verify_receipt_signer(&receipt, &expected_pubkey));

        // Wrong pubkey should fail
        let wrong_pubkey = [0x42u8; 64];
        assert!(!verify_receipt_signer(&receipt, &wrong_pubkey));
    }
}
