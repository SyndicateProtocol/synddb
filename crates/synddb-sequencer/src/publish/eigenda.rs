//! `EigenDA` layer publisher

use super::PublishResult;
use crate::attestor::SignedBatch;
use crate::config::EigenDAConfig;
use anyhow::Result;

#[derive(Debug)]
pub struct EigenDAPublisher {
    _config: EigenDAConfig,
}

impl EigenDAPublisher {
    pub const fn new(config: EigenDAConfig) -> Self {
        Self { _config: config }
    }

    /// Publish batch to `EigenDA`
    pub async fn publish(&self, _batch: &SignedBatch) -> Result<PublishResult> {
        // TODO: Use eigenda-rust to publish blob
        // TODO: Return blob reference

        Ok(PublishResult {
            layer: "eigenda".to_string(),
            success: false,
            reference: None,
            error: Some("Not implemented".to_string()),
        })
    }
}
