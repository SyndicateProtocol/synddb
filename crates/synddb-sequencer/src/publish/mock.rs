// //! Mock publisher for testing
//
// use async_trait::async_trait;
// use std::{
//     collections::HashMap,
//     sync::{Arc, Mutex},
// };
//
// use crate::{
//     publish::traits::{PublishError, PublishResult, StoragePublisher},
//     signer::MessageSigner,
// };
// use synddb_shared::types::message::{SignedBatch, SignedMessage};
//
// /// Internal state for `MockPublisher`
// #[derive(Debug, Default)]
// struct MockState {
//     messages: HashMap<u64, SignedMessage>,
//     batches: HashMap<u64, SignedBatch>,
//     saved_sequence: Option<u64>,
//     fail_on_publish: bool,
// }
//
// /// In-memory publisher for testing
// #[derive(Debug)]
// pub struct MockPublisher {
//     state: Mutex<MockState>,
//     /// Signer for creating batch signatures
//     signer: Arc<MessageSigner>,
// }
//
// impl MockPublisher {
//     pub fn new(signer: Arc<MessageSigner>) -> Self {
//         Self {
//             state: Mutex::new(MockState::default()),
//             signer,
//         }
//     }
//
//     /// Set whether publish operations should fail
//     pub fn set_fail_on_publish(&self, fail: bool) {
//         self.state.lock().unwrap().fail_on_publish = fail;
//     }
// }
//
// #[async_trait]
// impl StoragePublisher for MockPublisher {
//     fn name(&self) -> &str {
//         "mock"
//     }
//
//     async fn publish(&self, message: &SignedMessage) -> PublishResult {
//         if self.state.lock().unwrap().fail_on_publish {
//             return PublishResult::failure("mock", "Simulated failure");
//         }
//
//         // Wrap single message in a batch with proper batch signature
//         let messages_vec = vec![message.clone()];
//
//         // Serialize messages for content hash (using SHA-256 for CBOR format)
//         let messages_json = match serde_json::to_vec(&messages_vec) {
//             Ok(json) => json,
//             Err(e) => {
//                 return PublishResult::failure("mock", format!("Serialization error: {e}"));
//             }
//         };
//
//         // Compute content hash and sign with CBOR format (64-byte signature)
//         let content_hash = MessageSigner::compute_content_hash(&messages_json);
//         let batch_signature = match self
//             .signer
//             .sign_batch_cbor(message.sequence, message.sequence, &content_hash)
//             .await
//         {
//             Ok(sig) => sig.to_hex_prefixed(),
//             Err(e) => {
//                 return PublishResult::failure("mock", format!("Signing error: {e}"));
//             }
//         };
//
//         let batch = SignedBatch {
//             start_sequence: message.sequence,
//             end_sequence: message.sequence,
//             messages: messages_vec,
//             batch_signature,
//             signer: format!("0x{}", hex::encode(self.signer.public_key())),
//             created_at: message.timestamp,
//             content_hash,
//         };
//
//         self.publish_batch(&batch).await
//     }
//
//     async fn publish_batch(&self, batch: &SignedBatch) -> PublishResult {
//         // Sanity check: verify batch signature before publishing
//         if let Err(e) = batch.verify_batch_signature() {
//             return PublishResult::failure("mock", format!("Signature verification failed: {e}"));
//         }
//
//         let mut state = self.state.lock().unwrap();
//
//         if state.fail_on_publish {
//             return PublishResult::failure("mock", "Simulated failure");
//         }
//
//         // Store batch and index individual messages
//         state.batches.insert(batch.start_sequence, batch.clone());
//         for msg in &batch.messages {
//             state.messages.insert(msg.sequence, msg.clone());
//         }
//
//         PublishResult::success(
//             "mock",
//             format!(
//                 "mock://batch/{}_{}",
//                 batch.start_sequence, batch.end_sequence
//             ),
//         )
//     }
//
//     async fn get(&self, sequence: u64) -> Result<Option<SignedMessage>, PublishError> {
//         let state = self.state.lock().unwrap();
//         Ok(state.messages.get(&sequence).cloned())
//     }
//
//     async fn get_batch(&self, start_sequence: u64) -> Result<Option<SignedBatch>, PublishError> {
//         let state = self.state.lock().unwrap();
//         Ok(state.batches.get(&start_sequence).cloned())
//     }
//
//     async fn get_latest_sequence(&self) -> Result<Option<u64>, PublishError> {
//         let state = self.state.lock().unwrap();
//
//         let msg_max = state.messages.keys().max().copied();
//         let batch_max = state.batches.values().map(|b| b.end_sequence).max();
//
//         Ok(match (msg_max, batch_max) {
//             (Some(m), Some(b)) => Some(m.max(b)),
//             (Some(m), None) => Some(m),
//             (None, Some(b)) => Some(b),
//             (None, None) => None,
//         })
//     }
//
//     async fn save_state(&self, sequence: u64) -> Result<(), PublishError> {
//         self.state.lock().unwrap().saved_sequence = Some(sequence);
//         Ok(())
//     }
//
//     async fn load_state(&self) -> Result<Option<u64>, PublishError> {
//         Ok(self.state.lock().unwrap().saved_sequence)
//     }
// }
//
// #[cfg(test)]
// mod tests {
//     use super::*;
//     use alloy::{
//         primitives::{keccak256, B256},
//         signers::{local::PrivateKeySigner, SignerSync},
//     };
//     use synddb_shared::types::cbor::{
//         error::CborError,
//         message::{CborMessageType, CborSignedMessage},
//     };
//
//     const TEST_PRIVATE_KEY: &str =
//         "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
//
//     fn test_signer() -> Arc<MessageSigner> {
//         Arc::new(MessageSigner::new(TEST_PRIVATE_KEY).unwrap())
//     }
//
//     fn private_signer() -> PrivateKeySigner {
//         TEST_PRIVATE_KEY.parse().unwrap()
//     }
//
//     /// Sign for COSE (64-byte signature)
//     fn sign_cose(signer: &PrivateKeySigner, data: &[u8]) -> Result<[u8; 64], CborError> {
//         let hash = keccak256(data);
//         let sig = signer
//             .sign_hash_sync(&B256::from(hash))
//             .map_err(|e| CborError::Signing(e.to_string()))?;
//
//         let mut result = [0u8; 64];
//         result[..32].copy_from_slice(&sig.r().to_be_bytes::<32>());
//         result[32..].copy_from_slice(&sig.s().to_be_bytes::<32>());
//         Ok(result)
//     }
//
//     /// Get signer's 64-byte uncompressed public key (without 0x04 prefix)
//     fn signer_pubkey(signer: &PrivateKeySigner) -> [u8; 64] {
//         let pubkey = signer.credential().verifying_key().to_encoded_point(false);
//         let bytes = pubkey.as_bytes();
//         let mut result = [0u8; 64];
//         result.copy_from_slice(&bytes[1..65]);
//         result
//     }
//
//     /// Create a COSE-signed message for testing
//     fn create_signed_message(sequence: u64, timestamp: u64) -> SignedMessage {
//         let signer = private_signer();
//         let pubkey = signer_pubkey(&signer);
//         let payload = b"test payload";
//
//         let cbor_msg = CborSignedMessage::new(
//             sequence,
//             timestamp,
//             CborMessageType::Changeset,
//             payload.to_vec(),
//             pubkey,
//             |data| sign_cose(&signer, data),
//         )
//         .unwrap();
//
//         cbor_msg.to_signed_message(&pubkey).unwrap()
//     }
//
//     #[tokio::test]
//     async fn test_mock_publisher_roundtrip() {
//         let signer = test_signer();
//         let publisher = MockPublisher::new(Arc::clone(&signer));
//
//         // Create a COSE-signed message
//         let message = create_signed_message(42, 1700000000);
//
//         // Publish
//         let result = publisher.publish(&message).await;
//         assert!(result.success);
//         assert_eq!(result.reference, Some("mock://batch/42_42".to_string()));
//
//         // Retrieve
//         let retrieved = publisher.get(42).await.unwrap();
//         assert!(retrieved.is_some());
//         assert_eq!(retrieved.unwrap().sequence, 42);
//
//         // Non-existent
//         let missing = publisher.get(999).await.unwrap();
//         assert!(missing.is_none());
//     }
//
//     #[tokio::test]
//     async fn test_mock_publisher_state() {
//         let publisher = MockPublisher::new(test_signer());
//
//         // Initially no state
//         assert!(publisher.load_state().await.unwrap().is_none());
//
//         // Save state
//         publisher.save_state(100).await.unwrap();
//
//         // Load state
//         assert_eq!(publisher.load_state().await.unwrap(), Some(100));
//     }
//
//     #[tokio::test]
//     async fn test_mock_publisher_failure() {
//         let publisher = MockPublisher::new(test_signer());
//         publisher.set_fail_on_publish(true);
//
//         let message = create_signed_message(1, 1700000000);
//
//         let result = publisher.publish(&message).await;
//         assert!(!result.success);
//         assert!(result.error.is_some());
//     }
//
//     #[tokio::test]
//     async fn test_mock_publisher_batch() {
//         let signer = test_signer();
//         let publisher = MockPublisher::new(Arc::clone(&signer));
//
//         // Create COSE-signed messages
//         let msg1 = create_signed_message(1, 1700000000);
//         let msg2 = create_signed_message(2, 1700000001);
//         let messages = vec![msg1, msg2];
//
//         // Create batch with proper content hash
//         let content_hash = [0x42u8; 32]; // Placeholder hash for test
//
//         // Sign batch payload
//         let mut payload_data = Vec::new();
//         payload_data.extend_from_slice(&1u64.to_be_bytes());
//         payload_data.extend_from_slice(&2u64.to_be_bytes());
//         payload_data.extend_from_slice(&content_hash);
//         let batch_payload = keccak256(&payload_data);
//         let batch_hash = keccak256(batch_payload);
//
//         let pk_signer = private_signer();
//         let pubkey = signer_pubkey(&pk_signer);
//         let batch_sig = pk_signer.sign_hash_sync(&B256::from(batch_hash)).unwrap();
//         let mut sig_bytes = [0u8; 64];
//         sig_bytes[..32].copy_from_slice(&batch_sig.r().to_be_bytes::<32>());
//         sig_bytes[32..].copy_from_slice(&batch_sig.s().to_be_bytes::<32>());
//
//         let batch = SignedBatch {
//             start_sequence: 1,
//             end_sequence: 2,
//             messages,
//             batch_signature: format!("0x{}", hex::encode(sig_bytes)),
//             signer: format!("0x{}", hex::encode(pubkey)),
//             created_at: 1700000002,
//             content_hash,
//         };
//
//         // Publish batch
//         let result = publisher.publish_batch(&batch).await;
//         assert!(result.success);
//         assert_eq!(result.reference, Some("mock://batch/1_2".to_string()));
//
//         // Retrieve individual messages
//         let msg1 = publisher.get(1).await.unwrap();
//         assert!(msg1.is_some());
//         assert_eq!(msg1.unwrap().sequence, 1);
//
//         let msg2 = publisher.get(2).await.unwrap();
//         assert!(msg2.is_some());
//         assert_eq!(msg2.unwrap().sequence, 2);
//
//         // Retrieve batch
//         let retrieved_batch = publisher.get_batch(1).await.unwrap();
//         assert!(retrieved_batch.is_some());
//         let retrieved_batch = retrieved_batch.unwrap();
//         assert_eq!(retrieved_batch.start_sequence, 1);
//         assert_eq!(retrieved_batch.end_sequence, 2);
//         assert_eq!(retrieved_batch.messages.len(), 2);
//
//         // Latest sequence should be 2
//         assert_eq!(publisher.get_latest_sequence().await.unwrap(), Some(2));
//     }
// }
