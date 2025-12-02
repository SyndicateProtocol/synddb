//! Celestia DA layer publisher

use crate::publish::traits::PublishResult;
use synddb_shared::types::message::SignedMessage;

#[derive(Debug)]
pub struct CelestiaPublisher {
    _config: CelestiaConfig,
}

#[derive(Debug)]
pub struct CelestiaConfig {}

impl CelestiaPublisher {
    pub const fn new(config: CelestiaConfig) -> Self {
        Self { _config: config }
    }

    /// Publish batch to Celestia
    pub async fn publish(&self, _message: &SignedMessage) -> PublishResult {
        // TODO: Use celestia-client to publish blob
        // TODO: Return blob ID as reference

        PublishResult::default()
    }
}
