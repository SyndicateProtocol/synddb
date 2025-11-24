//! Example: Deposit Handler
//!
//! This example demonstrates how to implement a custom `MessageHandler`
//! to process deposit events from a Bridge contract.
//!
//! Run with:
//! ```bash
//! cargo run --example deposit_handler
//! ```

use alloy::{primitives::B256, rpc::types::Log, sol_types::SolEvent};
use anyhow::Result;
use clap::Parser;
use std::sync::Arc;
use synddb_chain_monitor::{
    config::ChainMonitorConfig, events::Deposit, handler::MessageHandler, monitor::ChainMonitor,
};
use tracing::{error, info};

/// Example deposit handler that processes deposit events.
///
/// In a real application, this would:
/// - Store deposits in a database
/// - Credit user accounts
/// - Emit notifications
/// - Update application state
#[derive(Debug, Default)]
pub struct DepositHandler {
    // In a real app, you'd have database connections, etc.
    processed_count: std::sync::atomic::AtomicU64,
}

impl DepositHandler {
    pub fn get_processed_count(&self) -> u64 {
        self.processed_count
            .load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl MessageHandler for DepositHandler {
    async fn handle_event(&self, log: &Log) -> Result<bool> {
        // Decode the deposit event from the log
        let deposit = Deposit::decode_log(&log.inner)?;

        info!(
            from = %format!("{:#x}", deposit.from),
            to = %format!("{:#x}", deposit.to),
            amount = %deposit.amount,
            tx_hash = ?log.transaction_hash,
            block_number = ?log.block_number,
            "Processing deposit"
        );

        // In a real application, you would:
        // 1. Validate the deposit (check amounts, addresses, etc.)
        // 2. Store in database
        // 3. Credit the user's account
        // 4. Emit notifications

        // Example validation
        if deposit.amount == alloy::primitives::U256::ZERO {
            error!("Received deposit with zero amount, skipping");
            return Ok(false); // Don't mark as processed
        }

        // Simulate processing
        self.processed_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        info!(
            total_processed = self.get_processed_count(),
            "Deposit processed successfully"
        );

        Ok(true) // Mark as processed
    }

    fn event_signature(&self) -> Option<B256> {
        // Only process Deposit events
        Some(Deposit::SIGNATURE_HASH)
    }

    async fn on_start(&self) -> Result<()> {
        info!("DepositHandler started - ready to process deposits");
        Ok(())
    }

    async fn on_stop(&self) -> Result<()> {
        info!(
            total_processed = self.get_processed_count(),
            "DepositHandler stopped"
        );
        Ok(())
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

    info!("Starting deposit handler example");

    // Parse configuration from environment variables or CLI args
    let mut config = ChainMonitorConfig::parse();

    // Override event signature to only monitor Deposit events
    config.event_signature = Some(Deposit::SIGNATURE_HASH);

    // Override event store path for this example
    config.event_store_path = "./deposit_events.db".to_string();

    info!(
        ws_urls = ?config.ws_urls,
        contract_address = %format!("{:#x}", config.contract_address),
        start_block = config.start_block,
        "Configuration loaded"
    );

    // Create handler
    let handler = Arc::new(DepositHandler::default());

    // Create and run monitor
    let mut monitor = ChainMonitor::new(config, handler).await?;

    info!("Chain monitor initialized - starting event processing");

    // Run indefinitely
    monitor.run().await?;

    Ok(())
}
