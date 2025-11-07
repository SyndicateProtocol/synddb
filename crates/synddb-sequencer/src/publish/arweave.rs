//! Arweave storage publisher

use super::PublishResult;
use crate::attestor::SignedBatch;
use crate::config::ArweaveConfig;
use anyhow::Result;

pub struct ArweavePublisher {
    config: ArweaveConfig,
}

impl ArweavePublisher {
    pub fn new(config: ArweaveConfig) -> Self {
        Self { config }
    }

    /// Publish batch to Arweave
    pub async fn publish(&self, batch: &SignedBatch) -> Result<PublishResult> {
        // TODO: Use arweave-rs to upload data
        // TODO: Return transaction ID as reference

        Ok(PublishResult {
            layer: "arweave".to_string(),
            success: false,
            reference: None,
            error: Some("Not implemented".to_string()),
        })
    }
}
