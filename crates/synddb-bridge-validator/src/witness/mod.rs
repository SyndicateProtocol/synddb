//! Witness Validator implementation
//!
//! Witness Validators provide independent verification of messages
//! by watching for MessageInitialized events and re-verifying the
//! message data fetched from storage.

mod processor;

use std::sync::Arc;
use std::time::Duration;

use alloy::primitives::keccak256;
use alloy::providers::{Provider, ProviderBuilder, WsConnect};
use alloy::rpc::types::Filter;
use anyhow::{Context, Result};
use futures::StreamExt;
use tokio::sync::watch;
use tracing::{debug, error, info, warn};

use crate::bridge::IMessageBridge;
use crate::bridge::BridgeClient;
use crate::signing::MessageSigner;
use crate::state::MessageStore;
use crate::storage::StorageFetcher;
use crate::validation::ValidationPipeline;
use crate::ValidatorConfig;

pub use processor::MessageProcessor;

/// Witness Validator that watches for MessageInitialized events
/// and independently verifies and signs messages.
pub struct WitnessValidator {
    config: ValidatorConfig,
    bridge_client: Arc<BridgeClient>,
    signer: Arc<MessageSigner>,
    pipeline: Arc<ValidationPipeline>,
    storage_fetcher: Arc<StorageFetcher>,
    message_store: Arc<MessageStore>,
    shutdown_rx: watch::Receiver<bool>,
}

impl WitnessValidator {
    pub fn new(
        config: ValidatorConfig,
        bridge_client: Arc<BridgeClient>,
        signer: Arc<MessageSigner>,
        pipeline: Arc<ValidationPipeline>,
        storage_fetcher: Arc<StorageFetcher>,
        message_store: Arc<MessageStore>,
        shutdown_rx: watch::Receiver<bool>,
    ) -> Self {
        Self {
            config,
            bridge_client,
            signer,
            pipeline,
            storage_fetcher,
            message_store,
            shutdown_rx,
        }
    }

    /// Run the witness validator event loop
    pub async fn run(&mut self) -> Result<()> {
        info!(
            validator_address = %self.signer.address(),
            bridge = %self.config.bridge_address,
            "Starting witness validator"
        );

        loop {
            // Check for shutdown before starting event loop
            if *self.shutdown_rx.borrow() {
                info!("Shutdown signal received, stopping witness validator");
                break;
            }

            // Run the event loop until it completes or errors
            match self.run_event_loop().await {
                Ok(()) => {
                    info!("Event loop completed, restarting...");
                }
                Err(e) => {
                    error!(error = %e, "Event loop error, restarting after delay...");
                    // Wait for either shutdown or delay
                    tokio::select! {
                        _ = self.shutdown_rx.changed() => {
                            if *self.shutdown_rx.borrow() {
                                info!("Shutdown signal received during restart delay");
                                break;
                            }
                        }
                        _ = tokio::time::sleep(Duration::from_secs(5)) => {}
                    }
                }
            }
        }

        Ok(())
    }

    async fn run_event_loop(&self) -> Result<()> {
        let ws_url = self
            .config
            .ws_url
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("WS_URL is required for witness validator"))?;

        info!(ws_url = %ws_url, "Connecting to WebSocket provider");

        let ws = WsConnect::new(ws_url);
        let provider = ProviderBuilder::new().connect_ws(ws).await?;

        // Create filter for MessageInitialized events
        // MessageInitialized(bytes32 indexed messageId, bytes32 indexed domain, address indexed primaryValidator, string messageType, string storageRef)
        let event_signature = keccak256(
            "MessageInitialized(bytes32,bytes32,address,string,string)"
        );
        let filter = Filter::new()
            .address(self.config.bridge_address)
            .event_signature(event_signature);

        info!("Subscribing to MessageInitialized events");
        let sub = provider.subscribe_logs(&filter).await?;
        let mut stream = sub.into_stream();

        info!("Listening for MessageInitialized events...");

        while let Some(log) = stream.next().await {
            if *self.shutdown_rx.borrow() {
                break;
            }

            match self.process_log(&log).await {
                Ok(()) => {}
                Err(e) => {
                    warn!(error = %e, "Failed to process event");
                }
            }
        }

        Ok(())
    }

    async fn process_log(&self, log: &alloy::rpc::types::Log) -> Result<()> {
        // Decode the MessageInitialized event
        let event = log
            .log_decode::<IMessageBridge::MessageInitialized>()
            .context("Failed to decode MessageInitialized event")?;

        let message_id: [u8; 32] = event.inner.messageId.into();
        let domain: [u8; 32] = event.inner.domain.into();
        let _primary_validator = event.inner.primaryValidator;
        let message_type = &event.inner.messageType;
        let storage_ref = &event.inner.storageRef;

        info!(
            message_id = %hex::encode(message_id),
            domain = %hex::encode(domain),
            message_type = %message_type,
            storage_ref = %storage_ref,
            "Received MessageInitialized event"
        );

        // Check if we've already processed this message
        if self.message_store.is_processed(&message_id)? {
            debug!(message_id = %hex::encode(message_id), "Message already processed, skipping");
            return Ok(());
        }

        // Check if we've already signed this message
        let already_signed = self
            .bridge_client
            .has_validator_signed(message_id, self.signer.address())
            .await?;

        if already_signed {
            debug!(message_id = %hex::encode(message_id), "Already signed this message, skipping");
            self.message_store.mark_processed(&message_id)?;
            return Ok(());
        }

        // Process the message
        let processor = MessageProcessor::new(
            self.bridge_client.clone(),
            self.signer.clone(),
            self.pipeline.clone(),
            self.storage_fetcher.clone(),
        );

        match processor.process(message_id, storage_ref).await {
            Ok(()) => {
                info!(message_id = %hex::encode(message_id), "Successfully processed and signed message");
                self.message_store.mark_processed(&message_id)?;
            }
            Err(e) => {
                warn!(
                    message_id = %hex::encode(message_id),
                    error = %e,
                    "Failed to process message"
                );
                // Don't mark as processed so we can retry
            }
        }

        Ok(())
    }
}
