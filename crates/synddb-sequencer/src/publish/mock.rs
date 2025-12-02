//! Mock publisher for testing

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Mutex;

use crate::publish::traits::{DAPublisher, PublishError, PublishResult};
use synddb_shared::types::message::SignedMessage;

/// In-memory publisher for testing
#[derive(Debug, Default)]
pub struct MockPublisher {
    messages: Mutex<HashMap<u64, SignedMessage>>,
    state: Mutex<Option<u64>>,
    pub fail_on_publish: Mutex<bool>,
}

impl MockPublisher {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl DAPublisher for MockPublisher {
    fn name(&self) -> &str {
        "mock"
    }

    async fn publish(&self, message: &SignedMessage) -> PublishResult {
        if *self.fail_on_publish.lock().unwrap() {
            return PublishResult::failure("mock", "Simulated failure");
        }

        let mut messages = self.messages.lock().unwrap();
        messages.insert(message.sequence, message.clone());
        PublishResult::success("mock", format!("mock://{}", message.sequence))
    }

    async fn get(&self, sequence: u64) -> Result<Option<SignedMessage>, PublishError> {
        let messages = self.messages.lock().unwrap();
        Ok(messages.get(&sequence).cloned())
    }

    async fn get_latest_sequence(&self) -> Result<Option<u64>, PublishError> {
        let messages = self.messages.lock().unwrap();
        Ok(messages.keys().max().copied())
    }

    async fn save_state(&self, sequence: u64) -> Result<(), PublishError> {
        *self.state.lock().unwrap() = Some(sequence);
        Ok(())
    }

    async fn load_state(&self) -> Result<Option<u64>, PublishError> {
        Ok(*self.state.lock().unwrap())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use synddb_shared::types::message::MessageType;

    #[tokio::test]
    async fn test_mock_publisher_roundtrip() {
        let publisher = MockPublisher::new();

        let message = SignedMessage {
            sequence: 42,
            timestamp: 1700000000,
            message_type: MessageType::Changeset,
            payload: b"test payload".to_vec(),
            message_hash: "0x1234".to_string(),
            signature: "0xabcd".to_string(),
            signer: "0x5678".to_string(),
        };

        // Publish
        let result = publisher.publish(&message).await;
        assert!(result.success);
        assert_eq!(result.reference, Some("mock://42".to_string()));

        // Retrieve
        let retrieved = publisher.get(42).await.unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().sequence, 42);

        // Non-existent
        let missing = publisher.get(999).await.unwrap();
        assert!(missing.is_none());
    }

    #[tokio::test]
    async fn test_mock_publisher_state() {
        let publisher = MockPublisher::new();

        // Initially no state
        assert!(publisher.load_state().await.unwrap().is_none());

        // Save state
        publisher.save_state(100).await.unwrap();

        // Load state
        assert_eq!(publisher.load_state().await.unwrap(), Some(100));
    }

    #[tokio::test]
    async fn test_mock_publisher_failure() {
        let publisher = MockPublisher::new();
        *publisher.fail_on_publish.lock().unwrap() = true;

        let message = SignedMessage {
            sequence: 1,
            timestamp: 1700000000,
            message_type: MessageType::Changeset,
            payload: vec![],
            message_hash: "0x".to_string(),
            signature: "0x".to_string(),
            signer: "0x".to_string(),
        };

        let result = publisher.publish(&message).await;
        assert!(!result.success);
        assert!(result.error.is_some());
    }
}
