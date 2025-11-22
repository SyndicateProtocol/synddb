//! Publishing orchestration to multiple DA layers

pub mod arweave;
pub mod celestia;
pub mod eigenda;
pub mod ipfs;
pub mod manifest;
pub mod retry;

pub use manifest::PublishManifest;

use crate::attestor::SignedBatch;
use crate::config::PublishConfig;
use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Result of publishing to a DA layer
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishResult {
    pub layer: String,
    pub success: bool,
    pub reference: Option<String>, // CID, blob ID, etc.
    pub error: Option<String>,
}

/// Multi-layer publisher
#[derive(Debug)]
pub struct Publisher {
    _config: PublishConfig,
}

impl Publisher {
    pub const fn new(config: PublishConfig) -> Self {
        Self { _config: config }
    }

    /// Publish batch to all configured DA layers
    pub async fn publish(&self, _batch: SignedBatch) -> Result<Vec<PublishResult>> {
        // TODO: Publish to all configured layers in parallel
        // 1. Celestia
        // 2. EigenDA
        // 3. IPFS
        // 4. Arweave

        Ok(vec![])
    }
}
