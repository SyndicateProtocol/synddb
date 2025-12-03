//! Mock publisher for testing

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Mutex;

use crate::publish::traits::{DAPublisher, PublishError, PublishResult};
use synddb_shared::types::message::{SignedBatch, SignedMessage};

/// In-memory publisher for testing
#[derive(Debug, Default)]
pub struct MockPublisher {
    messages: Mutex<HashMap<u64, SignedMessage>>,
    batches: Mutex<HashMap<u64, SignedBatch>>,
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

    async fn publish_batch(&self, batch: &SignedBatch) -> PublishResult {
        if *self.fail_on_publish.lock().unwrap() {
            return PublishResult::failure("mock", "Simulated failure");
        }

        // Store batch
        let mut batches = self.batches.lock().unwrap();
        batches.insert(batch.start_sequence, batch.clone());

        // Also index individual messages for get()
        let mut messages = self.messages.lock().unwrap();
        for msg in &batch.messages {
            messages.insert(msg.sequence, msg.clone());
        }

        PublishResult::success(
            "mock",
            format!(
                "mock://batch/{}_{}",
                batch.start_sequence, batch.end_sequence
            ),
        )
    }

    async fn get(&self, sequence: u64) -> Result<Option<SignedMessage>, PublishError> {
        let messages = self.messages.lock().unwrap();
        Ok(messages.get(&sequence).cloned())
    }

    async fn get_batch(&self, start_sequence: u64) -> Result<Option<SignedBatch>, PublishError> {
        let batches = self.batches.lock().unwrap();
        Ok(batches.get(&start_sequence).cloned())
    }

    async fn get_latest_sequence(&self) -> Result<Option<u64>, PublishError> {
        let messages = self.messages.lock().unwrap();
        let batches = self.batches.lock().unwrap();

        let msg_max = messages.keys().max().copied();
        let batch_max = batches.values().map(|b| b.end_sequence).max();

        Ok(match (msg_max, batch_max) {
            (Some(m), Some(b)) => Some(m.max(b)),
            (Some(m), None) => Some(m),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        })
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

    #[tokio::test]
    async fn test_mock_publisher_batch() {
        let publisher = MockPublisher::new();

        let messages = vec![
            SignedMessage {
                sequence: 1,
                timestamp: 1700000000,
                message_type: MessageType::Changeset,
                payload: b"msg1".to_vec(),
                message_hash: "0x1".to_string(),
                signature: "0xsig1".to_string(),
                signer: "0xsigner".to_string(),
            },
            SignedMessage {
                sequence: 2,
                timestamp: 1700000001,
                message_type: MessageType::Changeset,
                payload: b"msg2".to_vec(),
                message_hash: "0x2".to_string(),
                signature: "0xsig2".to_string(),
                signer: "0xsigner".to_string(),
            },
        ];

        let batch = SignedBatch {
            start_sequence: 1,
            end_sequence: 2,
            messages,
            batch_signature: "0xbatchsig".to_string(),
            signer: "0xsigner".to_string(),
            created_at: 1700000002,
        };

        // Publish batch
        let result = publisher.publish_batch(&batch).await;
        assert!(result.success);
        assert_eq!(result.reference, Some("mock://batch/1_2".to_string()));

        // Retrieve individual messages
        let msg1 = publisher.get(1).await.unwrap();
        assert!(msg1.is_some());
        assert_eq!(msg1.unwrap().sequence, 1);

        let msg2 = publisher.get(2).await.unwrap();
        assert!(msg2.is_some());
        assert_eq!(msg2.unwrap().sequence, 2);

        // Retrieve batch
        let retrieved_batch = publisher.get_batch(1).await.unwrap();
        assert!(retrieved_batch.is_some());
        let retrieved_batch = retrieved_batch.unwrap();
        assert_eq!(retrieved_batch.start_sequence, 1);
        assert_eq!(retrieved_batch.end_sequence, 2);
        assert_eq!(retrieved_batch.messages.len(), 2);

        // Latest sequence should be 2
        assert_eq!(publisher.get_latest_sequence().await.unwrap(), Some(2));
    }
}
