//! Attestation and signing module

pub mod compressor;
pub mod key_manager;
pub mod signer;

pub use compressor::Compressor;
pub use key_manager::KeyManager;
pub use signer::BatchSigner;

use serde::{Deserialize, Serialize};

/// Signed and compressed batch ready for publishing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignedBatch {
    /// Compressed payload data
    pub compressed_data: Vec<u8>,
    /// Signature over compressed data
    pub signature: Vec<u8>,
    /// Ethereum address of signer
    pub signer_address: String,
    /// Sequence number
    pub sequence: u64,
}
