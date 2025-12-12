//! CBOR batch type for storing multiple signed messages

use super::{error::CborError, message::CborSignedMessage};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};

/// Current batch format version
pub(super) const BATCH_VERSION: u8 = 1;

/// CBOR batch containing multiple `COSE_Sign1` messages
///
/// This is the top-level container stored in GCS. Each batch contains:
/// - Version number for format evolution
/// - Sequence range (start to end, inclusive)
/// - Multiple signed messages
/// - Content hash for cross-system addressing
/// - Batch-level signature covering all messages
#[derive(Debug, Clone)]
pub struct CborBatch {
    /// Format version (currently 1)
    pub version: u8,
    /// First sequence number in batch (inclusive)
    pub start_sequence: u64,
    /// Last sequence number in batch (inclusive)
    pub end_sequence: u64,
    /// Unix timestamp when batch was created
    pub created_at: u64,
    /// SHA-256 hash of serialized messages (for content addressing)
    pub content_hash: [u8; 32],
    /// Signed messages in this batch
    pub messages: Vec<CborSignedMessage>,
    /// 64-byte batch signature (covers all messages)
    pub batch_signature: [u8; 64],
    /// 20-byte Ethereum address of signer
    pub signer: [u8; 20],
}

/// Internal serialization format for `CborBatch`
#[derive(Serialize, Deserialize)]
struct CborBatchWire {
    #[serde(rename = "v")]
    version: u8,
    #[serde(rename = "s")]
    start_sequence: u64,
    #[serde(rename = "e")]
    end_sequence: u64,
    #[serde(rename = "t")]
    created_at: u64,
    #[serde(rename = "h", with = "serde_bytes")]
    content_hash: Vec<u8>,
    #[serde(rename = "m")]
    messages: Vec<serde_bytes::ByteBuf>,
    #[serde(rename = "sig", with = "serde_bytes")]
    batch_signature: Vec<u8>,
    #[serde(rename = "addr", with = "serde_bytes")]
    signer: Vec<u8>,
}

impl CborBatch {
    /// Create a new batch from messages, computing content hash and signing
    ///
    /// # Arguments
    /// * `messages` - Signed messages to include in the batch
    /// * `created_at` - Unix timestamp for batch creation
    /// * `signer_address` - 20-byte Ethereum address
    /// * `sign_fn` - Function to sign the batch payload, returns 64-byte signature
    pub fn new<F>(
        messages: Vec<CborSignedMessage>,
        created_at: u64,
        signer_address: [u8; 20],
        sign_fn: F,
    ) -> Result<Self, CborError>
    where
        F: FnOnce(&[u8]) -> Result<[u8; 64], CborError>,
    {
        if messages.is_empty() {
            return Err(CborError::InvalidBatch("Batch cannot be empty".to_string()));
        }

        // Extract sequence range
        let start_sequence = messages
            .first()
            .map(|m| m.sequence())
            .transpose()?
            .ok_or_else(|| {
                CborError::InvalidBatch("Cannot get sequence from message".to_string())
            })?;

        let end_sequence = messages
            .last()
            .map(|m| m.sequence())
            .transpose()?
            .ok_or_else(|| {
                CborError::InvalidBatch("Cannot get sequence from message".to_string())
            })?;

        // Compute content hash over all message bytes
        let content_hash = Self::compute_content_hash(&messages);

        // Compute the signing payload
        let signing_payload =
            Self::compute_signing_payload(start_sequence, end_sequence, &content_hash);

        // Sign it
        let batch_signature = sign_fn(&signing_payload)?;

        Ok(Self {
            version: BATCH_VERSION,
            start_sequence,
            end_sequence,
            created_at,
            content_hash,
            messages,
            batch_signature,
            signer: signer_address,
        })
    }

    /// Compute SHA-256 hash of all message bytes concatenated
    fn compute_content_hash(messages: &[CborSignedMessage]) -> [u8; 32] {
        let mut hasher = Sha256::new();
        for msg in messages {
            hasher.update(msg.as_bytes());
        }
        hasher.finalize().into()
    }

    /// Compute the payload that gets signed for batch verification
    ///
    /// Format: `keccak256(start_sequence || end_sequence || content_hash)`
    pub fn compute_signing_payload(
        start_sequence: u64,
        end_sequence: u64,
        content_hash: &[u8; 32],
    ) -> [u8; 32] {
        use alloy::primitives::keccak256;

        let mut data = Vec::with_capacity(8 + 8 + 32);
        data.extend_from_slice(&start_sequence.to_be_bytes());
        data.extend_from_slice(&end_sequence.to_be_bytes());
        data.extend_from_slice(content_hash);

        keccak256(&data).0
    }

    /// Serialize to CBOR bytes (uncompressed)
    pub fn to_cbor(&self) -> Result<Vec<u8>, CborError> {
        let wire = CborBatchWire {
            version: self.version,
            start_sequence: self.start_sequence,
            end_sequence: self.end_sequence,
            created_at: self.created_at,
            content_hash: self.content_hash.to_vec(),
            messages: self
                .messages
                .iter()
                .map(|m| serde_bytes::ByteBuf::from(m.as_bytes().to_vec()))
                .collect(),
            batch_signature: self.batch_signature.to_vec(),
            signer: self.signer.to_vec(),
        };

        let mut buf = Vec::new();
        ciborium::into_writer(&wire, &mut buf)?;
        Ok(buf)
    }

    /// Serialize to CBOR + zstd compressed bytes
    pub fn to_cbor_zstd(&self) -> Result<Vec<u8>, CborError> {
        let cbor = self.to_cbor()?;
        let mut encoder = zstd::Encoder::new(Vec::new(), 3)?;
        encoder.write_all(&cbor)?;
        encoder.finish().map_err(CborError::from)
    }

    /// Parse from CBOR bytes (uncompressed)
    pub fn from_cbor(bytes: &[u8]) -> Result<Self, CborError> {
        let wire: CborBatchWire = ciborium::from_reader(bytes)?;
        Self::from_wire(wire)
    }

    /// Parse from CBOR + zstd compressed bytes
    pub fn from_cbor_zstd(bytes: &[u8]) -> Result<Self, CborError> {
        let mut decoder = zstd::Decoder::new(bytes)?;
        let mut cbor = Vec::new();
        decoder.read_to_end(&mut cbor)?;
        Self::from_cbor(&cbor)
    }

    /// Convert wire format to `CborBatch`
    fn from_wire(wire: CborBatchWire) -> Result<Self, CborError> {
        let content_hash: [u8; 32] = wire
            .content_hash
            .try_into()
            .map_err(|_| CborError::InvalidBatch("Invalid content hash length".to_string()))?;

        let batch_signature: [u8; 64] = wire
            .batch_signature
            .try_into()
            .map_err(|_| CborError::InvalidBatch("Invalid batch signature length".to_string()))?;

        let signer: [u8; 20] = wire
            .signer
            .try_into()
            .map_err(|_| CborError::InvalidBatch("Invalid signer length".to_string()))?;

        let messages: Vec<CborSignedMessage> = wire
            .messages
            .into_iter()
            .map(|b| CborSignedMessage::from_bytes(b.into_vec()))
            .collect();

        Ok(Self {
            version: wire.version,
            start_sequence: wire.start_sequence,
            end_sequence: wire.end_sequence,
            created_at: wire.created_at,
            content_hash,
            messages,
            batch_signature,
            signer,
        })
    }

    /// Verify batch signature
    pub fn verify_batch_signature(&self) -> Result<(), CborError> {
        use alloy::primitives::{keccak256, Signature, B256, U256};

        // Recompute content hash
        let computed_hash = Self::compute_content_hash(&self.messages);
        if computed_hash != self.content_hash {
            return Err(CborError::SignatureVerification(
                "Content hash mismatch".to_string(),
            ));
        }

        // Compute the signing payload
        let payload = Self::compute_signing_payload(
            self.start_sequence,
            self.end_sequence,
            &self.content_hash,
        );

        // Hash the payload
        let message_hash = keccak256(payload);

        // Try both recovery IDs
        for v in [false, true] {
            let sig = Signature::new(
                U256::from_be_slice(&self.batch_signature[..32]),
                U256::from_be_slice(&self.batch_signature[32..]),
                v,
            );

            if let Ok(recovered) = sig.recover_address_from_prehash(&B256::from(message_hash)) {
                if recovered.as_slice() == self.signer {
                    return Ok(());
                }
            }
        }

        Err(CborError::SignatureVerification(
            "Batch signature verification failed".to_string(),
        ))
    }

    /// Verify batch and all message signatures
    pub fn verify_all_signatures(&self) -> Result<(), CborError> {
        // Verify batch signature first
        self.verify_batch_signature()?;

        // Verify each message signature
        for (i, msg) in self.messages.iter().enumerate() {
            msg.verify_and_parse(&self.signer).map_err(|e| {
                CborError::SignatureVerification(format!(
                    "Message {} verification failed: {}",
                    i, e
                ))
            })?;
        }

        Ok(())
    }

    /// Get content hash as hex string (for logging and Arweave tags)
    pub fn content_hash_hex(&self) -> String {
        format!("0x{}", hex::encode(self.content_hash))
    }

    /// Get total uncompressed size of all messages
    pub fn total_message_bytes(&self) -> usize {
        self.messages.iter().map(|m| m.size()).sum()
    }

    /// Get number of messages in this batch
    pub const fn message_count(&self) -> usize {
        self.messages.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_content_hash_deterministic() {
        let msg1 = CborSignedMessage::from_bytes(vec![1, 2, 3]);
        let msg2 = CborSignedMessage::from_bytes(vec![4, 5, 6]);

        let hash1 = CborBatch::compute_content_hash(&[msg1.clone(), msg2.clone()]);
        let hash2 = CborBatch::compute_content_hash(&[msg1, msg2]);

        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_compute_content_hash_order_matters() {
        let msg1 = CborSignedMessage::from_bytes(vec![1, 2, 3]);
        let msg2 = CborSignedMessage::from_bytes(vec![4, 5, 6]);

        let hash1 = CborBatch::compute_content_hash(&[msg1.clone(), msg2.clone()]);
        let hash2 = CborBatch::compute_content_hash(&[msg2, msg1]);

        assert_ne!(hash1, hash2);
    }
}
