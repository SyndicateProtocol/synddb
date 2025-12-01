//! IPFS storage publisher

use crate::publish::traits::PublishResult;
use synddb_shared::types::SignedMessage;

#[derive(Debug)]
pub struct IpfsPublisher {
    _config: IpfsConfig,
}

#[derive(Debug)]
pub struct IpfsConfig {}

impl IpfsPublisher {
    pub const fn new(config: IpfsConfig) -> Self {
        Self { _config: config }
    }

    /// Publish batch to IPFS
    pub async fn publish(&self, _message: &SignedMessage) -> PublishResult {
        // TODO: Use ipfs-api to add content
        // TODO: Return CID as reference

        PublishResult::default()
    }
}
