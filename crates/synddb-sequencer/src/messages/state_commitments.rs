//! Signed state commitments for validators

use super::degradation::SystemStatus;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateCommitment {
    /// Hash of the state update (changeset or snapshot)
    pub state_hash: String,
    /// Current system status
    pub status: SystemStatus,
    /// Sequence number
    pub sequence: u64,
    /// Timestamp
    pub timestamp: u64,
    /// TEE signature
    pub signature: Vec<u8>,
}

impl StateCommitment {
    pub const fn new(
        state_hash: String,
        status: SystemStatus,
        sequence: u64,
        timestamp: u64,
    ) -> Self {
        Self {
            state_hash,
            status,
            sequence,
            timestamp,
            signature: vec![],
        }
    }

    /// Sign the commitment with TEE key
    pub fn sign(&mut self, signature: Vec<u8>) {
        self.signature = signature;
    }
}
