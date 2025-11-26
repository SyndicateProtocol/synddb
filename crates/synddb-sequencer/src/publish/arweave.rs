//! Arweave storage publisher

use crate::inbox::SignedMessage;
use crate::publish::traits::PublishResult;

#[derive(Debug)]
pub struct ArweavePublisher {
    _config: ArweaveConfig,
}

#[derive(Debug)]
pub struct ArweaveConfig {}

impl ArweavePublisher {
    pub const fn new(config: ArweaveConfig) -> Self {
        Self { _config: config }
    }

    /// Publish batch to Arweave
    pub async fn publish(&self, _message: &SignedMessage) -> PublishResult {
        // TODO: Use arweave-rs to upload data
        // TODO: Return transaction ID as reference

        PublishResult::default()
    }
}
