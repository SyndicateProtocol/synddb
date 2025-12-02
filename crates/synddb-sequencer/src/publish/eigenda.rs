//! `EigenDA` layer publisher

use crate::publish::traits::PublishResult;
use synddb_shared::types::message::SignedMessage;

#[derive(Debug)]
pub struct EigenDAPublisher {
    _config: EigenDAConfig,
}

#[derive(Debug)]
pub struct EigenDAConfig {}

impl EigenDAPublisher {
    pub const fn new(config: EigenDAConfig) -> Self {
        Self { _config: config }
    }

    /// Publish batch to `EigenDA`
    pub async fn publish(&self, _message: &SignedMessage) -> PublishResult {
        // TODO: Use eigenda-rust to publish blob
        // TODO: Return blob reference

        PublishResult::default()
    }
}
