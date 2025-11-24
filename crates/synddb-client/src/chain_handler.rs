//! Chain event handler for processing blockchain deposits
//!
//! This module implements a `MessageHandler` that listens to blockchain events
//! (deposits) from a Bridge contract and sends them via a channel to be inserted
//! into the local `SQLite` database.

use anyhow::Result;
use tracing::{error, info, warn};

#[cfg(feature = "chain-monitor")]
use alloy::{primitives::B256, rpc::types::Log, sol_types::SolEvent};
#[cfg(feature = "chain-monitor")]
use crossbeam_channel::Sender;
#[cfg(feature = "chain-monitor")]
use synddb_chain_monitor::{events::Deposit, handler::MessageHandler};

/// Deposit data extracted from blockchain events
#[cfg(feature = "chain-monitor")]
#[derive(Debug, Clone)]
pub struct DepositData {
    pub tx_hash: String,
    pub block_number: u64,
    pub log_index: Option<u64>,
    pub from: String,
    pub to: String,
    pub amount: String,
    pub data: Vec<u8>,
}

/// Handler for processing deposit events from the blockchain
///
/// This handler receives deposit events from the chain monitor and sends them
/// via a channel to be inserted into the database by the main thread.
#[cfg(feature = "chain-monitor")]
#[derive(Debug)]
pub struct DepositHandler {
    /// Channel to send deposit data to main thread
    tx: Sender<DepositData>,
    /// Counter for processed deposits
    processed_count: std::sync::atomic::AtomicU64,
}

#[cfg(feature = "chain-monitor")]
impl DepositHandler {
    /// Create a new deposit handler
    ///
    /// # Arguments
    ///
    /// * `tx` - Channel to send deposit data to main thread for database insertion
    pub const fn new(tx: Sender<DepositData>) -> Self {
        Self {
            tx,
            processed_count: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// Get the number of deposits processed
    pub fn get_processed_count(&self) -> u64 {
        self.processed_count
            .load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[cfg(feature = "chain-monitor")]
#[async_trait::async_trait]
impl MessageHandler for DepositHandler {
    async fn handle_event(&self, log: &Log) -> Result<bool> {
        // Decode the deposit event
        let deposit = match Deposit::decode_log(&log.inner) {
            Ok(d) => d,
            Err(e) => {
                error!("Failed to decode deposit event: {}", e);
                return Ok(false); // Don't mark as processed if we can't decode
            }
        };

        // Extract transaction hash and block info
        let tx_hash = log
            .transaction_hash
            .map_or_else(|| "unknown".to_string(), |h| format!("{:#x}", h));
        let block_number = log.block_number.unwrap_or(0);
        let log_index = log.log_index;

        info!(
            tx_hash = %tx_hash,
            block_number = block_number,
            from = %format!("{:#x}", deposit.from),
            to = %format!("{:#x}", deposit.to),
            amount = %deposit.amount,
            "Processing deposit event"
        );

        // Validate deposit
        if deposit.amount == alloy::primitives::U256::ZERO {
            warn!("Deposit with zero amount detected - skipping");
            return Ok(false); // Don't mark as processed
        }

        // Send deposit data via channel to main thread
        let deposit_data = DepositData {
            tx_hash,
            block_number,
            log_index,
            from: format!("{:#x}", deposit.from),
            to: format!("{:#x}", deposit.to),
            amount: deposit.amount.to_string(),
            data: deposit.data.data.to_vec(),
        };

        match self.tx.send(deposit_data) {
            Ok(_) => {
                self.processed_count
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                info!(
                    total_processed = self.get_processed_count(),
                    "Deposit queued for database insertion"
                );
                Ok(true) // Successfully processed
            }
            Err(e) => {
                error!("Failed to send deposit to main thread: {}", e);
                Ok(false) // Don't mark as processed if send failed
            }
        }
    }

    fn event_signature(&self) -> Option<B256> {
        // Only process Deposit events
        Some(Deposit::SIGNATURE_HASH)
    }

    async fn on_start(&self) -> Result<()> {
        info!("DepositHandler started - monitoring blockchain for deposits");
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
