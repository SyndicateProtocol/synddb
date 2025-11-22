//! IPFS storage publisher

use super::PublishResult;
use crate::attestor::SignedBatch;
use crate::config::IpfsConfig;
use anyhow::Result;

#[derive(Debug)]
pub struct IpfsPublisher {
    _config: IpfsConfig,
}

impl IpfsPublisher {
    pub const fn new(config: IpfsConfig) -> Self {
        Self { _config: config }
    }

    /// Publish batch to IPFS
    pub async fn publish(&self, _batch: &SignedBatch) -> Result<PublishResult> {
        // TODO: Use ipfs-api to add content
        // TODO: Return CID as reference

        Ok(PublishResult {
            layer: "ipfs".to_string(),
            success: false,
            reference: None,
            error: Some("Not implemented".to_string()),
        })
    }
}
