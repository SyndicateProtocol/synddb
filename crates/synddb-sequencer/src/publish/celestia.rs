//! Celestia DA layer publisher

use super::PublishResult;
use crate::attestor::SignedBatch;
use crate::config::CelestiaConfig;
use anyhow::Result;

pub struct CelestiaPublisher {
    _config: CelestiaConfig,
}

impl CelestiaPublisher {
    pub fn new(config: CelestiaConfig) -> Self {
        Self { _config: config }
    }

    /// Publish batch to Celestia
    pub async fn publish(&self, _batch: &SignedBatch) -> Result<PublishResult> {
        // TODO: Use celestia-client to publish blob
        // TODO: Return blob ID as reference

        Ok(PublishResult {
            layer: "celestia".to_string(),
            success: false,
            reference: None,
            error: Some("Not implemented".to_string()),
        })
    }
}
