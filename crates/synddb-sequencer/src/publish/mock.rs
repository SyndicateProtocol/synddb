//! Mock publisher for testing

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::publish::traits::{DAPublisher, PublishError, PublishResult};
use crate::signer::MessageSigner;
use synddb_shared::types::message::{SignedBatch, SignedMessage};

/// Internal state for `MockPublisher`
#[derive(Debug, Default)]
struct MockState {
    messages: HashMap<u64, SignedMessage>,
    batches: HashMap<u64, SignedBatch>,
    saved_sequence: Option<u64>,
    fail_on_publish: bool,
}

/// In-memory publisher for testing
#[derive(Debug)]
pub struct MockPublisher {
    state: Mutex<MockState>,
    /// Signer for creating batch signatures
    signer: Arc<MessageSigner>,
}

impl MockPublisher {
    pub fn new(signer: Arc<MessageSigner>) -> Self {
        Self {
            state: Mutex::new(MockState::default()),
            signer,
        }
    }

    /// Set whether publish operations should fail
    pub fn set_fail_on_publish(&self, fail: bool) {
        self.state.lock().unwrap().fail_on_publish = fail;
    }
}

#[async_trait]
impl DAPublisher for MockPublisher {
    fn name(&self) -> &str {
        "mock"
    }

    async fn publish(&self, message: &SignedMessage) -> PublishResult {
        if self.state.lock().unwrap().fail_on_publish {
            return PublishResult::failure("mock", "Simulated failure");
        }

        // Wrap single message in a batch with proper batch signature
        let messages_vec = vec![message.clone()];

        // Serialize messages for hashing
        let messages_json = match serde_json::to_vec(&messages_vec) {
            Ok(json) => json,
            Err(e) => {
                return PublishResult::failure("mock", format!("Serialization error: {e}"));
            }
        };

        // Compute messages hash and sign the batch
        let messages_hash = MessageSigner::compute_messages_hash(&messages_json);
        let batch_signature = match self
            .signer
            .sign_batch(message.sequence, message.sequence, messages_hash)
            .await
        {
            Ok(sig) => sig.to_hex_prefixed(),
            Err(e) => {
                return PublishResult::failure("mock", format!("Signing error: {e}"));
            }
        };

        let batch = SignedBatch {
            start_sequence: message.sequence,
            end_sequence: message.sequence,
            messages: messages_vec,
            batch_signature,
            signer: format!("{:?}", self.signer.address()),
            created_at: message.timestamp,
        };

        self.publish_batch(&batch).await
    }

    async fn publish_batch(&self, batch: &SignedBatch) -> PublishResult {
        // Sanity check: verify batch signature before publishing
        if let Err(e) = batch.verify_batch_signature() {
            return PublishResult::failure("mock", format!("Signature verification failed: {e}"));
        }

        let mut state = self.state.lock().unwrap();

        if state.fail_on_publish {
            return PublishResult::failure("mock", "Simulated failure");
        }

        // Store batch and index individual messages
        state.batches.insert(batch.start_sequence, batch.clone());
        for msg in &batch.messages {
            state.messages.insert(msg.sequence, msg.clone());
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
        let state = self.state.lock().unwrap();
        Ok(state.messages.get(&sequence).cloned())
    }

    async fn get_batch(&self, start_sequence: u64) -> Result<Option<SignedBatch>, PublishError> {
        let state = self.state.lock().unwrap();
        Ok(state.batches.get(&start_sequence).cloned())
    }

    async fn get_latest_sequence(&self) -> Result<Option<u64>, PublishError> {
        let state = self.state.lock().unwrap();

        let msg_max = state.messages.keys().max().copied();
        let batch_max = state.batches.values().map(|b| b.end_sequence).max();

        Ok(match (msg_max, batch_max) {
            (Some(m), Some(b)) => Some(m.max(b)),
            (Some(m), None) => Some(m),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        })
    }

    async fn save_state(&self, sequence: u64) -> Result<(), PublishError> {
        self.state.lock().unwrap().saved_sequence = Some(sequence);
        Ok(())
    }

    async fn load_state(&self) -> Result<Option<u64>, PublishError> {
        Ok(self.state.lock().unwrap().saved_sequence)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::keccak256;
    use synddb_shared::types::message::MessageType;

    const TEST_PRIVATE_KEY: &str =
        "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

    fn test_signer() -> Arc<MessageSigner> {
        Arc::new(MessageSigner::new(TEST_PRIVATE_KEY).unwrap())
    }

    /// Create a properly signed message for testing
    async fn create_signed_message(
        signer: &MessageSigner,
        sequence: u64,
        timestamp: u64,
    ) -> SignedMessage {
        let payload = b"test payload";
        let message_hash = keccak256(payload);
        let signing_payload =
            SignedMessage::compute_signing_payload(sequence, timestamp, message_hash);
        let signature = signer.sign(signing_payload).await.unwrap();

        SignedMessage {
            sequence,
            timestamp,
            message_type: MessageType::Changeset,
            payload: payload.to_vec(),
            message_hash: format!("0x{}", hex::encode(message_hash)),
            signature: signature.to_hex_prefixed(),
            signer: format!("{:?}", signer.address()),
        }
    }

    #[tokio::test]
    async fn test_mock_publisher_roundtrip() {
        let signer = test_signer();
        let publisher = MockPublisher::new(Arc::clone(&signer));

        // Create a properly signed message
        let message = create_signed_message(&signer, 42, 1700000000).await;

        // Publish
        let result = publisher.publish(&message).await;
        assert!(result.success);
        // Now returns batch reference since publish wraps in a batch
        assert_eq!(result.reference, Some("mock://batch/42_42".to_string()));

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
        let publisher = MockPublisher::new(test_signer());

        // Initially no state
        assert!(publisher.load_state().await.unwrap().is_none());

        // Save state
        publisher.save_state(100).await.unwrap();

        // Load state
        assert_eq!(publisher.load_state().await.unwrap(), Some(100));
    }

    #[tokio::test]
    async fn test_mock_publisher_failure() {
        let publisher = MockPublisher::new(test_signer());
        publisher.set_fail_on_publish(true);

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
        let signer = test_signer();
        let publisher = MockPublisher::new(Arc::clone(&signer));

        // Create properly signed messages
        let msg1 = create_signed_message(&signer, 1, 1700000000).await;
        let msg2 = create_signed_message(&signer, 2, 1700000001).await;
        let messages = vec![msg1, msg2];

        // Create properly signed batch
        let messages_hash = SignedBatch::compute_messages_hash(&messages).unwrap();
        let batch_payload = SignedBatch::compute_signing_payload(1, 2, messages_hash);
        let batch_sig = signer.sign(batch_payload).await.unwrap();

        let batch = SignedBatch {
            start_sequence: 1,
            end_sequence: 2,
            messages,
            batch_signature: batch_sig.to_hex_prefixed(),
            signer: format!("{:?}", signer.address()),
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
