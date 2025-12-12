//! Chain monitor service orchestration.
//!
//! This module provides the main `ChainMonitor` service that orchestrates
//! blockchain event monitoring using WebSocket or RPC polling.

use crate::{
    config::ChainMonitorConfig, eth_client::EthClient, event_store::EventStore,
    handler::MessageHandler,
};
use alloy::{
    primitives::Address,
    rpc::types::{Filter, Log},
};
use anyhow::Result;
use std::{fmt::Debug, sync::Arc, time::Duration};
use tokio::time::timeout;
use tracing::{debug, error, info, warn};

/// Chain monitor service that listens to blockchain events.
///
/// The monitor supports two modes:
/// - **WebSocket mode**: Real-time event streaming (preferred)
/// - **RPC polling mode**: Periodic polling for events (fallback)
#[derive(Debug)]
pub struct ChainMonitor {
    eth_client: EthClient,
    contract_address: Address,
    filter: Filter,
    handler: Arc<dyn MessageHandler>,
    event_store: EventStore,
}

impl ChainMonitor {
    /// Create a new chain monitor.
    ///
    /// # Arguments
    ///
    /// * `config` - Monitor configuration
    /// * `handler` - Event handler implementation
    ///
    /// # Returns
    ///
    /// A configured `ChainMonitor` ready to run.
    pub async fn new(config: ChainMonitorConfig, handler: Arc<dyn MessageHandler>) -> Result<Self> {
        info!(
            contract = %format!("{:#x}", config.contract_address),
            start_block = config.start_block,
            "Initializing chain monitor"
        );

        // Create EthClient with configured parameters
        let eth_client = EthClient::new(
            config.ws_urls.clone(),
            config.monitor_request_timeout,
            config.get_logs_timeout,
            config.channel_size,
            config.retry_interval,
        )
        .await;

        // Build filter - use handler's event signature if available
        let event_sig = config.event_signature.or_else(|| handler.event_signature());

        let mut filter = Filter::new()
            .address(config.contract_address)
            .from_block(config.start_block);

        if let Some(sig) = event_sig {
            info!(signature = %format!("{:#x}", sig), "Filtering for specific event signature");
            filter = filter.event_signature(sig);
        } else {
            info!("Monitoring all events from contract");
        }

        // Create event store for idempotency tracking
        let event_store = EventStore::new(&config.event_store_path)?;

        // Call handler initialization
        handler.on_start().await?;

        Ok(Self {
            eth_client,
            contract_address: config.contract_address,
            filter,
            handler,
            event_store,
        })
    }

    /// Run the chain monitor.
    ///
    /// This method will try WebSocket subscription first, and fall back to
    /// RPC polling if WebSocket is not supported by the provider.
    ///
    /// This method runs indefinitely until an error occurs or the process is terminated.
    pub async fn run(&mut self) -> Result<()> {
        info!(
            contract = %format!("{:#x}", self.contract_address),
            "Starting chain monitor"
        );

        // Try WebSocket first, fallback to RPC polling
        match self.eth_client.try_subscribe_logs(&self.filter).await {
            Ok(_) => {
                info!("WebSocket subscription supported - using real-time monitoring");
                self.run_ws_monitor().await
            }
            Err(e) => {
                warn!(%e, "WebSocket not supported - falling back to RPC polling");
                self.run_rpc_monitor().await
            }
        }
    }

    /// Run in WebSocket subscription mode.
    ///
    /// This provides real-time event delivery with low latency.
    async fn run_ws_monitor(&self) -> Result<()> {
        let mut sub = self.eth_client.subscribe_logs(&self.filter).await;
        info!("WebSocket subscription active");

        loop {
            let heartbeat_timeout = Duration::from_secs(30);

            match timeout(heartbeat_timeout, sub.recv()).await {
                Ok(Ok(log)) => {
                    if let Err(e) = self.process_event(&log).await {
                        error!(%e, "Failed to process event");
                    }
                }
                Ok(Err(e)) => {
                    error!(%e, "Subscription error, recreating...");
                    sub = self.eth_client.subscribe_logs(&self.filter).await;
                    info!("WebSocket subscription recreated");
                }
                Err(_) => {
                    debug!("Heartbeat: No events in last 30s");
                }
            }
        }
    }

    /// Run in RPC polling mode.
    ///
    /// This polls the RPC endpoint periodically for new events.
    async fn run_rpc_monitor(&self) -> Result<()> {
        let poll_interval = Duration::from_secs(5);
        let polling_window = 100; // blocks per poll

        // Resume from last processed block or use filter's start block
        let start_block_from_filter = self.filter.get_from_block().unwrap_or(0);

        let mut current_block = self
            .event_store
            .get_last_processed_block()?
            .unwrap_or(start_block_from_filter);

        info!(
            starting_block = current_block,
            poll_interval_secs = poll_interval.as_secs(),
            "RPC polling mode active"
        );

        loop {
            match self.poll_for_events(current_block, polling_window).await {
                Ok((logs, end_block)) => {
                    if !logs.is_empty() {
                        info!(
                            num_logs = logs.len(),
                            block_range = format!("{}-{}", current_block, end_block),
                            "Received events from RPC"
                        );
                    }

                    for log in logs {
                        if let Err(e) = self.process_event(&log).await {
                            error!(%e, "Failed to process event");
                        }
                    }

                    current_block = end_block;
                    self.event_store.set_last_processed_block(end_block)?;
                }
                Err(e) => {
                    error!(%e, "Error polling for logs");
                }
            }

            tokio::time::sleep(poll_interval).await;
        }
    }

    /// Poll for events in a specific block range.
    async fn poll_for_events(&self, start_block: u64, window: u64) -> Result<(Vec<Log>, u64)> {
        let latest_block = self
            .eth_client
            .get_latest_block()
            .await
            .ok_or_else(|| anyhow::anyhow!("Failed to get latest block"))?;

        let latest_block_number = latest_block.header.number;
        let end_block = std::cmp::min(start_block + window, latest_block_number);

        if end_block <= start_block {
            return Ok((vec![], start_block));
        }

        debug!(
            start_block = start_block,
            end_block = end_block,
            "Polling for events"
        );

        let logs = self
            .eth_client
            .get_logs(
                &self
                    .filter
                    .clone()
                    .from_block(start_block)
                    .to_block(end_block),
            )
            .await?;

        Ok((logs, end_block))
    }

    /// Process a single event.
    ///
    /// This method:
    /// 1. Checks idempotency (skip if already processed)
    /// 2. Delegates to the application-provided handler
    /// 3. Marks event as processed if handler returns true
    async fn process_event(&self, log: &Log) -> Result<()> {
        debug!(
            tx_hash = ?log.transaction_hash,
            block = ?log.block_number,
            "Received blockchain event"
        );

        // Check if already processed (idempotency)
        if let Some(tx_hash) = log.transaction_hash {
            if self.event_store.is_processed(&tx_hash)? {
                debug!("Event already processed, skipping");
                return Ok(());
            }
        }

        // Delegate to application-provided handler
        let handled = self.handler.handle_event(log).await?;

        if handled {
            // Mark as processed in the event store
            if let Some(tx_hash) = log.transaction_hash {
                self.event_store.mark_processed(
                    &tx_hash,
                    log.block_number.unwrap_or(0),
                    log.log_index,
                )?;
            }
        }

        Ok(())
    }
}

impl Drop for ChainMonitor {
    fn drop(&mut self) {
        // Call handler cleanup (best effort)
        let handler = self.handler.clone();
        tokio::spawn(async move {
            let _ = handler.on_stop().await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[derive(Debug)]
    struct TestHandler {
        count: Arc<AtomicU64>,
    }

    #[async_trait::async_trait]
    impl MessageHandler for TestHandler {
        async fn handle_event(&self, _log: &Log) -> Result<bool> {
            self.count.fetch_add(1, Ordering::SeqCst);
            Ok(true)
        }

        fn event_signature(&self) -> Option<alloy::primitives::B256> {
            None
        }
    }

    #[tokio::test]
    async fn test_handler_implementation() {
        // Test that the TestHandler works correctly
        let handler = TestHandler {
            count: Arc::new(AtomicU64::new(0)),
        };

        let log = Log::default();
        let result = handler.handle_event(&log).await;
        assert!(result.is_ok());
        assert!(result.unwrap());
        assert_eq!(handler.count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_event_store_integration() {
        // Test that EventStore works as expected in the monitor context
        let store = EventStore::new(":memory:").unwrap();
        let tx_hash = alloy::primitives::B256::from([0x42; 32]);

        assert!(!store.is_processed(&tx_hash).unwrap());
        store.mark_processed(&tx_hash, 100, Some(0)).unwrap();
        assert!(store.is_processed(&tx_hash).unwrap());
    }
}
