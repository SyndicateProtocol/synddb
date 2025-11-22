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
use std::sync::Arc;
use synddb_chain_monitor::{events::Deposit, ChainMonitor, ChainMonitorConfig, MessageHandler};
use tracing::{error, info};
use url::Url;

/// Example deposit handler that processes deposit events.
///
/// In a real application, this would:
/// - Store deposits in a database
/// - Credit user accounts
/// - Emit notifications
/// - Update application state
#[derive(Debug)]
pub struct DepositHandler {
    // In a real app, you'd have database connections, etc.
    processed_count: std::sync::atomic::AtomicU64,
}

impl Default for DepositHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl DepositHandler {
    pub const fn new() -> Self {
        Self {
            processed_count: std::sync::atomic::AtomicU64::new(0),
        }
    }

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

    // Example configuration - replace with your actual values
    // NOTE: This example will fail if you don't provide a real RPC URL and contract address
    let ws_url = std::env::var("WS_URL")
        .unwrap_or_else(|_| "wss://base-mainnet.g.alchemy.com/v2/YOUR_KEY".to_string());
    let contract_address = std::env::var("CONTRACT_ADDRESS")
        .unwrap_or_else(|_| "0x0000000000000000000000000000000000000000".to_string());
    let start_block: u64 = std::env::var("START_BLOCK")
        .unwrap_or_else(|_| "0".to_string())
        .parse()?;

    info!(
        ws_url = %ws_url,
        contract_address = %contract_address,
        start_block = start_block,
        "Configuration loaded"
    );

    // Create configuration
    let config = ChainMonitorConfig::new(
        vec![Url::parse(&ws_url)?],
        contract_address.parse()?,
        start_block,
    )
    .with_event_signature(Deposit::SIGNATURE_HASH)
    .with_event_store_path("./deposit_events.db".to_string());

    // Create handler
    let handler = Arc::new(DepositHandler::new());

    // Create and run monitor
    let mut monitor = ChainMonitor::new(config, handler).await?;

    info!("Chain monitor initialized - starting event processing");

    // Run indefinitely
    monitor.run().await?;

    Ok(())
}
