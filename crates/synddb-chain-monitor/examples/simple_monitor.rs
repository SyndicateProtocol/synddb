//! Example: Simple Monitor
//!
//! This example demonstrates the most basic usage of the chain monitor.
//! It monitors all events from a contract and logs them.
//!
//! Run with:
//! ```bash
//! WS_URL=wss://your-rpc-url CONTRACT_ADDRESS=0x... START_BLOCK=1000000 \
//!   cargo run --example simple_monitor
//! ```

use alloy::{primitives::B256, rpc::types::Log};
use anyhow::Result;
use clap::Parser;
use std::sync::Arc;
use synddb_chain_monitor::config::ChainMonitorConfig;
use synddb_chain_monitor::handler::MessageHandler;
use synddb_chain_monitor::monitor::ChainMonitor;
use tracing::info;

/// A simple handler that just logs all events it receives.
#[derive(Debug)]
struct SimpleHandler;

#[async_trait::async_trait]
impl MessageHandler for SimpleHandler {
    async fn handle_event(&self, log: &Log) -> Result<bool> {
        info!(
            tx_hash = ?log.transaction_hash,
            block_number = ?log.block_number,
            log_index = ?log.log_index,
            topics = ?log.topics().len(),
            data_len = log.data().data.len(),
            "Received event"
        );

        // Successfully processed
        Ok(true)
    }

    fn event_signature(&self) -> Option<B256> {
        // Monitor ALL events from the contract
        None
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .init();

    info!("Simple Chain Monitor Example");

    // Parse configuration from environment variables or CLI args
    let config = ChainMonitorConfig::parse();

    info!(
        ws_urls = ?config.ws_urls,
        contract = %format!("{:#x}", config.contract_address),
        start_block = config.start_block,
        "Starting monitor"
    );

    // Create handler
    let handler = Arc::new(SimpleHandler);

    // Create and run monitor
    let mut monitor = ChainMonitor::new(config, handler).await?;

    info!("Monitor initialized - listening for events...");

    // Run indefinitely (will use WebSocket if available, otherwise RPC polling)
    monitor.run().await?;

    Ok(())
}
