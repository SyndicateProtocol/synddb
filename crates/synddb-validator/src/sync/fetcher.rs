//! DA Fetcher trait for retrieving messages from data availability layers

use anyhow::Result;
use async_trait::async_trait;
use synddb_shared::types::SignedMessage;

/// Trait for fetching messages from a data availability layer
///
/// This is the read-only counterpart to `DAPublisher` in the sequencer.
/// Implementations fetch `SignedMessage` payloads that were published by the sequencer.
#[async_trait]
pub trait DAFetcher: Send + Sync + std::fmt::Debug {
    /// Name of this fetcher (e.g., "gcs", "celestia")
    fn name(&self) -> &str;

    /// Fetch a signed message by sequence number
    ///
    /// Returns `Ok(Some(message))` if found, `Ok(None)` if not found,
    /// or `Err` on network/parsing errors.
    async fn get(&self, sequence: u64) -> Result<Option<SignedMessage>>;

    /// Get the latest sequence number available from this DA layer
    ///
    /// Returns `Ok(Some(sequence))` if messages exist, `Ok(None)` if empty,
    /// or `Err` on network errors.
    async fn get_latest_sequence(&self) -> Result<Option<u64>>;
}
