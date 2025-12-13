//! Mock storage fetcher for testing

use crate::sync::fetcher::{BatchInfo, StorageFetcher};
use anyhow::Result;
use async_trait::async_trait;
use std::{collections::HashMap, sync::Mutex};
use synddb_shared::types::message::{SignedBatch, SignedMessage};

/// Mock fetcher for testing
///
/// Stores messages and batches in memory and allows test code to pre-populate
/// or dynamically add data. Supports both single-message and batch operations.
#[derive(Debug, Default)]
pub struct MockFetcher {
    /// Individual messages (for single-message mode or fallback)
    messages: Mutex<HashMap<u64, SignedMessage>>,
    /// Batches stored by `start_sequence`
    batches: Mutex<HashMap<u64, SignedBatch>>,
    /// Whether this fetcher should report batch support
    batch_mode: Mutex<bool>,
    /// If set, `get_latest_sequence` will fail with this error
    fail_latest: Mutex<Option<String>>,
    /// If set, `get(sequence)` will fail for these specific sequences
    fail_get: Mutex<HashMap<u64, String>>,
}

impl MockFetcher {
    /// Create a new empty mock fetcher (single-message mode by default)
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a new mock fetcher in batch mode
    pub fn new_batch_mode() -> Self {
        let fetcher = Self::default();
        *fetcher.batch_mode.lock().unwrap() = true;
        fetcher
    }

    /// Enable or disable batch mode
    pub fn set_batch_mode(&self, enabled: bool) {
        *self.batch_mode.lock().unwrap() = enabled;
    }

    /// Add a message to the mock store
    pub fn add_message(&self, message: SignedMessage) {
        self.messages
            .lock()
            .unwrap()
            .insert(message.sequence, message);
    }

    /// Add multiple messages
    pub fn add_messages(&self, messages: impl IntoIterator<Item = SignedMessage>) {
        let mut store = self.messages.lock().unwrap();
        for msg in messages {
            store.insert(msg.sequence, msg);
        }
    }

    /// Add a batch to the mock store
    pub fn add_batch(&self, batch: SignedBatch) {
        self.batches
            .lock()
            .unwrap()
            .insert(batch.start_sequence, batch);
    }

    /// Add multiple batches
    pub fn add_batches(&self, batches: impl IntoIterator<Item = SignedBatch>) {
        let mut store = self.batches.lock().unwrap();
        for batch in batches {
            store.insert(batch.start_sequence, batch);
        }
    }

    /// Clear all messages and batches
    pub fn clear(&self) {
        self.messages.lock().unwrap().clear();
        self.batches.lock().unwrap().clear();
    }

    /// Get the number of stored messages
    pub fn len(&self) -> usize {
        self.messages.lock().unwrap().len()
    }

    /// Get the number of stored batches
    pub fn batch_count(&self) -> usize {
        self.batches.lock().unwrap().len()
    }

    /// Check if empty (no messages and no batches)
    pub fn is_empty(&self) -> bool {
        self.messages.lock().unwrap().is_empty() && self.batches.lock().unwrap().is_empty()
    }

    /// Configure `get_latest_sequence` to fail with the given error
    pub fn set_fail_latest(&self, error: impl Into<String>) {
        *self.fail_latest.lock().unwrap() = Some(error.into());
    }

    /// Configure `get(sequence)` to fail for a specific sequence
    pub fn set_fail_get(&self, sequence: u64, error: impl Into<String>) {
        self.fail_get.lock().unwrap().insert(sequence, error.into());
    }

    /// Clear all failure configurations
    pub fn clear_failures(&self) {
        *self.fail_latest.lock().unwrap() = None;
        self.fail_get.lock().unwrap().clear();
    }

    /// Find a message by searching batches (used when `batch_mode` is enabled)
    fn find_message_in_batches(&self, sequence: u64) -> Option<SignedMessage> {
        let batches = self.batches.lock().unwrap();
        for batch in batches.values() {
            if sequence >= batch.start_sequence && sequence <= batch.end_sequence {
                return batch
                    .messages
                    .iter()
                    .find(|m| m.sequence == sequence)
                    .cloned();
            }
        }
        None
    }
}

#[async_trait]
impl StorageFetcher for MockFetcher {
    fn name(&self) -> &str {
        "mock"
    }

    fn supports_batches(&self) -> bool {
        *self.batch_mode.lock().unwrap()
    }

    async fn get(&self, sequence: u64) -> Result<Option<SignedMessage>> {
        // Check if this sequence should fail
        if let Some(error) = self.fail_get.lock().unwrap().get(&sequence) {
            return Err(anyhow::anyhow!("{}", error));
        }

        // First try individual messages
        let value = self.messages.lock().unwrap().get(&sequence).cloned();
        if let Some(msg) = value {
            return Ok(Some(msg));
        }

        // Fall back to searching batches
        Ok(self.find_message_in_batches(sequence))
    }

    async fn get_latest_sequence(&self) -> Result<Option<u64>> {
        // Check if we should fail
        if let Some(error) = self.fail_latest.lock().unwrap().as_ref() {
            return Err(anyhow::anyhow!("{}", error));
        }

        // Get max from individual messages
        let msg_max = self.messages.lock().unwrap().keys().max().copied();

        // Get max from batches
        let batch_max = self
            .batches
            .lock()
            .unwrap()
            .values()
            .map(|b| b.end_sequence)
            .max();

        // Return the overall max
        Ok(match (msg_max, batch_max) {
            (Some(m), Some(b)) => Some(m.max(b)),
            (Some(m), None) => Some(m),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        })
    }

    async fn list_batches(&self) -> Result<Vec<BatchInfo>> {
        let batches = self.batches.lock().unwrap();
        let mut infos: Vec<BatchInfo> = batches
            .values()
            .map(|b| {
                BatchInfo::new(
                    b.start_sequence,
                    b.end_sequence,
                    b.start_sequence.to_string(),
                )
            })
            .collect();
        infos.sort_by_key(|b| b.start_sequence);
        Ok(infos)
    }

    async fn get_batch(&self, start_sequence: u64) -> Result<Option<SignedBatch>> {
        Ok(self.batches.lock().unwrap().get(&start_sequence).cloned())
    }

    async fn get_batch_by_path(&self, path: &str) -> Result<Option<SignedBatch>> {
        // For mock, path is just the start_sequence as a string
        let start_sequence: u64 = path
            .parse()
            .map_err(|e| anyhow::anyhow!("Invalid batch path: {e}"))?;
        self.get_batch(start_sequence).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use synddb_shared::types::message::MessageType;

    fn test_message(sequence: u64) -> SignedMessage {
        SignedMessage {
            sequence,
            timestamp: 1700000000 + sequence,
            message_type: MessageType::Changeset,
            payload: vec![1, 2, 3],
            message_hash: format!("0x{:064x}", sequence),
            signature: format!("0x{:0130x}", sequence),
            signer: "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266".to_string(),
            cose_protected_header: vec![],
        }
    }

    #[tokio::test]
    async fn test_mock_fetcher_empty() {
        let fetcher = MockFetcher::new();

        assert!(fetcher.is_empty());
        assert_eq!(fetcher.get_latest_sequence().await.unwrap(), None);
        assert_eq!(fetcher.get(0).await.unwrap(), None);
    }

    #[tokio::test]
    async fn test_mock_fetcher_add_message() {
        let fetcher = MockFetcher::new();
        let msg = test_message(42);

        fetcher.add_message(msg.clone());

        assert_eq!(fetcher.len(), 1);
        assert_eq!(fetcher.get_latest_sequence().await.unwrap(), Some(42));

        let retrieved = fetcher.get(42).await.unwrap().unwrap();
        assert_eq!(retrieved.sequence, 42);
    }

    #[tokio::test]
    async fn test_mock_fetcher_add_messages() {
        let fetcher = MockFetcher::new();
        let messages = vec![test_message(0), test_message(1), test_message(2)];

        fetcher.add_messages(messages);

        assert_eq!(fetcher.len(), 3);
        assert_eq!(fetcher.get_latest_sequence().await.unwrap(), Some(2));
    }

    #[tokio::test]
    async fn test_mock_fetcher_failure_injection() {
        let fetcher = MockFetcher::new();
        fetcher.add_message(test_message(0));

        // Test fail_latest
        fetcher.set_fail_latest("network error");
        assert!(fetcher.get_latest_sequence().await.is_err());

        // Test fail_get
        fetcher.set_fail_get(0, "read error");
        assert!(fetcher.get(0).await.is_err());

        // Clear failures
        fetcher.clear_failures();
        assert!(fetcher.get_latest_sequence().await.is_ok());
        assert!(fetcher.get(0).await.is_ok());
    }

    #[tokio::test]
    async fn test_mock_fetcher_not_found() {
        let fetcher = MockFetcher::new();
        fetcher.add_message(test_message(0));

        // Sequence 1 doesn't exist
        assert_eq!(fetcher.get(1).await.unwrap(), None);
    }

    fn test_batch(start: u64, end: u64) -> SignedBatch {
        let messages: Vec<SignedMessage> = (start..=end).map(test_message).collect();
        SignedBatch {
            start_sequence: start,
            end_sequence: end,
            messages,
            batch_signature: format!("0x{:0130x}", start),
            signer: "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266".to_string(),
            created_at: 1700000000 + start,
            content_hash: [0u8; 32],
        }
    }

    #[tokio::test]
    async fn test_mock_fetcher_batch_mode() {
        let fetcher = MockFetcher::new_batch_mode();

        assert!(fetcher.supports_batches());
        assert!(fetcher.is_empty());
    }

    #[tokio::test]
    async fn test_mock_fetcher_add_batch() {
        let fetcher = MockFetcher::new_batch_mode();
        let batch = test_batch(1, 5);

        fetcher.add_batch(batch);

        assert_eq!(fetcher.batch_count(), 1);
        assert_eq!(fetcher.get_latest_sequence().await.unwrap(), Some(5));

        // Can retrieve individual messages from batch
        let msg = fetcher.get(3).await.unwrap().unwrap();
        assert_eq!(msg.sequence, 3);

        // Message outside batch range not found
        assert_eq!(fetcher.get(6).await.unwrap(), None);
    }

    #[tokio::test]
    async fn test_mock_fetcher_list_batches() {
        let fetcher = MockFetcher::new_batch_mode();

        // Add batches out of order
        fetcher.add_batch(test_batch(10, 15));
        fetcher.add_batch(test_batch(1, 5));
        fetcher.add_batch(test_batch(20, 25));

        let batches = fetcher.list_batches().await.unwrap();
        assert_eq!(batches.len(), 3);

        // Should be sorted by start_sequence
        assert_eq!(batches[0].start_sequence, 1);
        assert_eq!(batches[0].end_sequence, 5);

        assert_eq!(batches[1].start_sequence, 10);
        assert_eq!(batches[1].end_sequence, 15);

        assert_eq!(batches[2].start_sequence, 20);
        assert_eq!(batches[2].end_sequence, 25);
    }

    #[tokio::test]
    async fn test_mock_fetcher_get_batch() {
        let fetcher = MockFetcher::new_batch_mode();
        fetcher.add_batch(test_batch(1, 5));
        fetcher.add_batch(test_batch(10, 15));

        // Get existing batch
        let batch = fetcher.get_batch(1).await.unwrap().unwrap();
        assert_eq!(batch.start_sequence, 1);
        assert_eq!(batch.end_sequence, 5);
        assert_eq!(batch.messages.len(), 5);

        // Get non-existing batch
        assert!(fetcher.get_batch(999).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_mock_fetcher_get_batch_by_path() {
        let fetcher = MockFetcher::new_batch_mode();
        fetcher.add_batch(test_batch(1, 5));

        // Path is just the start_sequence as string
        let batch = fetcher.get_batch_by_path("1").await.unwrap().unwrap();
        assert_eq!(batch.start_sequence, 1);
    }

    #[tokio::test]
    async fn test_mock_fetcher_mixed_messages_and_batches() {
        let fetcher = MockFetcher::new_batch_mode();

        // Add individual message
        fetcher.add_message(test_message(100));

        // Add batch
        fetcher.add_batch(test_batch(1, 5));

        // Latest should be max of both
        assert_eq!(fetcher.get_latest_sequence().await.unwrap(), Some(100));

        // Can get both individual and batch messages
        assert!(fetcher.get(100).await.unwrap().is_some());
        assert!(fetcher.get(3).await.unwrap().is_some());
    }
}
