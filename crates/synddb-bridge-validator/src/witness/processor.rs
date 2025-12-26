//! Message processor for Witness Validator
//!
//! Handles fetching message data from storage, validating, and signing.

use std::sync::Arc;

use anyhow::{Context, Result};
use sha3::{Digest, Keccak256};
use tracing::{debug, info};

use crate::{
    bridge::BridgeClient,
    signing::{compute_message_id, MessageSigner},
    storage::StorageFetcher,
    types::Message,
    validation::ValidationPipeline,
};

/// Processes a single message: fetch, validate, sign
pub struct MessageProcessor {
    bridge_client: Arc<BridgeClient>,
    signer: Arc<MessageSigner>,
    pipeline: Arc<ValidationPipeline>,
    storage_fetcher: Arc<StorageFetcher>,
}

impl MessageProcessor {
    pub fn new(
        bridge_client: Arc<BridgeClient>,
        signer: Arc<MessageSigner>,
        pipeline: Arc<ValidationPipeline>,
        storage_fetcher: Arc<StorageFetcher>,
    ) -> Self {
        Self {
            bridge_client,
            signer,
            pipeline,
            storage_fetcher,
        }
    }

    /// Process a message: fetch from storage, validate, sign, and submit
    pub async fn process(&self, message_id: [u8; 32], storage_ref: &str) -> Result<()> {
        // 1. Fetch message data from storage
        debug!(storage_ref = %storage_ref, "Fetching message from storage");
        let record = self
            .storage_fetcher
            .fetch(storage_ref)
            .await
            .context("Failed to fetch message from storage")?;

        // 2. Reconstruct the message
        let message = Message {
            id: message_id,
            message_type: record.message_type.clone(),
            calldata: record.calldata.clone(),
            metadata: record.metadata.clone(),
            metadata_hash: record.metadata_hash,
            nonce: record.nonce,
            timestamp: record.timestamp,
            domain: record.domain,
            value: record.value,
        };

        // 3. Verify message ID matches
        let computed_id = compute_message_id(
            &message.message_type,
            &message.calldata,
            &message.metadata_hash,
            message.nonce,
            message.timestamp,
            &message.domain,
        );

        if computed_id != message_id {
            anyhow::bail!(
                "Message ID mismatch: expected {}, computed {}",
                hex::encode(message_id),
                hex::encode(computed_id)
            );
        }

        // 4. Verify metadata hash
        let metadata_bytes = serde_json::to_vec(&message.metadata)?;
        let computed_hash: [u8; 32] = Keccak256::digest(&metadata_bytes).into();
        if computed_hash != message.metadata_hash {
            anyhow::bail!(
                "Metadata hash mismatch: expected {}, computed {}",
                hex::encode(message.metadata_hash),
                hex::encode(computed_hash)
            );
        }

        // 5. Validate message through pipeline
        debug!("Validating message through pipeline");
        self.pipeline
            .validate_witness(&message, &self.bridge_client)
            .await
            .context("Message validation failed")?;

        // 6. Sign the message
        debug!("Signing message");
        let signature = self
            .signer
            .sign_message(&message)
            .await
            .context("Failed to sign message")?;

        // 7. Submit signature to bridge
        info!(
            message_id = %hex::encode(message_id),
            signer = %self.signer.address(),
            "Submitting signature to bridge"
        );

        self.bridge_client
            .sign_message(message_id, &signature)
            .await
            .context("Failed to submit signature to bridge")?;

        Ok(())
    }
}
