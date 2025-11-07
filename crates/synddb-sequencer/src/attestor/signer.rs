//! Sign batches with Ethereum key

use super::{KeyManager, SignedBatch};
use crate::batch::BatchPayload;
use anyhow::Result;

pub struct BatchSigner {
    key_manager: KeyManager,
}

impl BatchSigner {
    pub fn new(key_manager: KeyManager) -> Self {
        Self { key_manager }
    }

    /// Sign a batch payload
    pub async fn sign_batch(&self, payload: BatchPayload, sequence: u64) -> Result<SignedBatch> {
        // TODO: Serialize and sign batch
        // 1. Compress data
        // 2. Sign compressed data
        // 3. Return SignedBatch

        Ok(SignedBatch {
            compressed_data: vec![],
            signature: vec![],
            signer_address: self.key_manager.address(),
            sequence,
        })
    }
}
