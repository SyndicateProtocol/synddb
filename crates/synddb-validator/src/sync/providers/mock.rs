//! Mock DA fetcher for testing

use crate::sync::fetcher::DAFetcher;
use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Mutex;
use synddb_shared::types::SignedMessage;

/// Mock fetcher for testing
///
/// Stores messages in memory and allows test code to pre-populate
/// or dynamically add messages.
#[derive(Debug, Default)]
pub struct MockFetcher {
    messages: Mutex<HashMap<u64, SignedMessage>>,
    /// If set, `get_latest_sequence` will fail with this error
    fail_latest: Mutex<Option<String>>,
    /// If set, `get(sequence)` will fail for these specific sequences
    fail_get: Mutex<HashMap<u64, String>>,
}

impl MockFetcher {
    /// Create a new empty mock fetcher
    pub fn new() -> Self {
        Self::default()
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

    /// Clear all messages
    pub fn clear(&self) {
        self.messages.lock().unwrap().clear();
    }

    /// Get the number of stored messages
    pub fn len(&self) -> usize {
        self.messages.lock().unwrap().len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.messages.lock().unwrap().is_empty()
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
}

#[async_trait]
impl DAFetcher for MockFetcher {
    fn name(&self) -> &str {
        "mock"
    }

    async fn get(&self, sequence: u64) -> Result<Option<SignedMessage>> {
        // Check if this sequence should fail
        if let Some(error) = self.fail_get.lock().unwrap().get(&sequence) {
            return Err(anyhow::anyhow!("{}", error));
        }

        Ok(self.messages.lock().unwrap().get(&sequence).cloned())
    }

    async fn get_latest_sequence(&self) -> Result<Option<u64>> {
        // Check if we should fail
        if let Some(error) = self.fail_latest.lock().unwrap().as_ref() {
            return Err(anyhow::anyhow!("{}", error));
        }

        Ok(self.messages.lock().unwrap().keys().max().copied())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use synddb_shared::types::MessageType;

    fn test_message(sequence: u64) -> SignedMessage {
        SignedMessage {
            sequence,
            timestamp: 1700000000 + sequence,
            message_type: MessageType::Changeset,
            payload: vec![1, 2, 3],
            message_hash: format!("0x{:064x}", sequence),
            signature: format!("0x{:0130x}", sequence),
            signer: "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266".to_string(),
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
}
