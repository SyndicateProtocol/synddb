//! Example: Multi-Event Handler
//!
//! This example demonstrates how to handle multiple event types from a Bridge contract.
//! It shows a more realistic production scenario with:
//! - Different handlers for different event types
//! - Event routing based on signatures
//! - Shared state management
//! - Error handling
//!
//! Run with:
//! ```bash
//! WS_URL=wss://your-rpc-url CONTRACT_ADDRESS=0x... START_BLOCK=1000000 \
//!   cargo run --example multi_event_handler
//! ```

use alloy::{primitives::B256, rpc::types::Log, sol_types::SolEvent};
use anyhow::{Context, Result};
use clap::Parser;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use synddb_chain_monitor::{
    events::{Deposit, StateSync, Withdrawal},
    ChainMonitor, ChainMonitorConfig, MessageHandler,
};
use tracing::{error, info, warn};

/// Shared statistics for all event handlers.
#[derive(Debug, Default)]
struct EventStats {
    deposits: AtomicU64,
    withdrawals: AtomicU64,
    state_syncs: AtomicU64,
    errors: AtomicU64,
}

impl EventStats {
    fn log_summary(&self) {
        info!(
            deposits = self.deposits.load(Ordering::Relaxed),
            withdrawals = self.withdrawals.load(Ordering::Relaxed),
            state_syncs = self.state_syncs.load(Ordering::Relaxed),
            errors = self.errors.load(Ordering::Relaxed),
            "Event processing statistics"
        );
    }
}

/// Multi-event handler that routes events to appropriate processors.
#[derive(Debug, Default)]
struct MultiEventHandler {
    stats: Arc<EventStats>,
}

impl MultiEventHandler {
    fn get_stats(&self) -> Arc<EventStats> {
        self.stats.clone()
    }

    async fn handle_deposit(&self, log: &Log) -> Result<bool> {
        let deposit = Deposit::decode_log(&log.inner).context("Failed to decode Deposit event")?;

        info!(
            from = %format!("{:#x}", deposit.from),
            to = %format!("{:#x}", deposit.to),
            amount = %deposit.amount,
            data_len = deposit.data.data.len(),
            "Deposit event"
        );

        // Validation
        if deposit.amount == alloy::primitives::U256::ZERO {
            warn!("Deposit with zero amount detected - skipping");
            return Ok(false); // Don't mark as processed
        }

        // In a real application, you would:
        // 1. Store deposit in database
        // 2. Credit user account
        // 3. Emit notification
        // 4. Update metrics

        self.stats.deposits.fetch_add(1, Ordering::Relaxed);
        Ok(true)
    }

    async fn handle_withdrawal(&self, log: &Log) -> Result<bool> {
        let withdrawal =
            Withdrawal::decode_log(&log.inner).context("Failed to decode Withdrawal event")?;

        info!(
            from = %format!("{:#x}", withdrawal.from),
            amount = %withdrawal.amount,
            recipient = %format!("{:#x}", withdrawal.recipient),
            "Withdrawal event"
        );

        // Validation
        if withdrawal.amount == alloy::primitives::U256::ZERO {
            warn!("Withdrawal with zero amount detected - skipping");
            return Ok(false);
        }

        // In a real application, you would:
        // 1. Verify withdrawal proof
        // 2. Execute withdrawal on L1
        // 3. Update user balance
        // 4. Log transaction

        self.stats.withdrawals.fetch_add(1, Ordering::Relaxed);
        Ok(true)
    }

    async fn handle_state_sync(&self, log: &Log) -> Result<bool> {
        let state_sync =
            StateSync::decode_log(&log.inner).context("Failed to decode StateSync event")?;

        info!(
            block_number = %state_sync.blockNumber,
            state_root = %format!("{:#x}", state_sync.stateRoot),
            proof_len = state_sync.proof.len(),
            "StateSync event"
        );

        // In a real application, you would:
        // 1. Verify state proof
        // 2. Update local state
        // 3. Sync with L2

        self.stats.state_syncs.fetch_add(1, Ordering::Relaxed);
        Ok(true)
    }
}

#[async_trait::async_trait]
impl MessageHandler for MultiEventHandler {
    async fn handle_event(&self, log: &Log) -> Result<bool> {
        // Route based on the first topic (event signature)
        let event_sig = log.topics().first().copied();

        let result = match event_sig {
            Some(sig) if sig == Deposit::SIGNATURE_HASH => self.handle_deposit(log).await,
            Some(sig) if sig == Withdrawal::SIGNATURE_HASH => self.handle_withdrawal(log).await,
            Some(sig) if sig == StateSync::SIGNATURE_HASH => self.handle_state_sync(log).await,
            Some(sig) => {
                warn!(signature = %format!("{:#x}", sig), "Unknown event signature");
                Ok(false) // Don't mark as processed
            }
            None => {
                warn!("Event with no topics");
                Ok(false)
            }
        };

        // Track errors
        if result.is_err() {
            self.stats.errors.fetch_add(1, Ordering::Relaxed);
            error!(error = ?result, "Error processing event");
        }

        result
    }

    fn event_signature(&self) -> Option<B256> {
        // Process ALL events from the contract
        // (we'll route internally based on signature)
        None
    }

    async fn on_start(&self) -> Result<()> {
        info!("MultiEventHandler started");
        Ok(())
    }

    async fn on_stop(&self) -> Result<()> {
        info!("MultiEventHandler stopping");
        self.stats.log_summary();
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging with more detailed output
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .with_target(true)
        .with_thread_ids(true)
        .init();

    info!("Multi-Event Handler Example");

    // Parse configuration from environment variables or CLI args
    let mut config = ChainMonitorConfig::parse();

    // Override event store path for this example
    config.event_store_path = "./bridge_events.db".to_string();

    info!(
        ws_urls = ?config.ws_urls,
        contract = %format!("{:#x}", config.contract_address),
        start_block = config.start_block,
        "Configuration loaded"
    );

    // Create handler
    let handler = Arc::new(MultiEventHandler::default());
    let stats = handler.get_stats();

    // Spawn a task to periodically log statistics
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            interval.tick().await;
            stats.log_summary();
        }
    });

    // Create and run monitor
    let mut monitor = ChainMonitor::new(config, handler).await?;

    info!("Monitor initialized - processing events from Bridge contract");

    // Run indefinitely
    monitor.run().await?;

    Ok(())
}
