//! Chain monitor integration for the prediction market
//!
//! This module provides handlers for blockchain events from the bridge contract:
//! - `DepositHandler`: Processes inbound USDC deposits from L1
//! - `WithdrawalHandler`: Confirms outbound USDC withdrawals on L1
//!
//! # Architecture
//!
//! The handlers use channels to communicate with the main thread, since
//! `rusqlite::Connection` is not `Sync`. Events are decoded in the async
//! handler and sent via channel, then inserted into the database by the
//! application's main thread.
//!
//! ```text
//! L1 Bridge Contract
//!        │
//!        ▼
//! ┌─────────────────┐
//! │  Chain Monitor  │  (watches for Deposit/Withdrawal events)
//! └────────┬────────┘
//!          │
//!          ▼
//! ┌─────────────────┐
//! │    Handlers     │  (decode events, send via channel)
//! └────────┬────────┘
//!          │ channel
//!          ▼
//! ┌─────────────────┐
//! │   Main Thread   │  (receives from channel, inserts into DB)
//! └────────┬────────┘
//!          │
//!          ▼
//! ┌─────────────────┐
//! │  `SQLite` Tables │  (`inbound_deposits`, `outbound_withdrawals`)
//! └─────────────────┘
//! ```

use alloy::{primitives::B256, rpc::types::Log, sol_types::SolEvent};
use anyhow::Result;
use crossbeam_channel::Sender;
use synddb_chain_monitor::{
    events::{Deposit, Withdrawal},
    handler::MessageHandler,
};
use tracing::{error, info, warn};

/// Deposit data extracted from blockchain events
#[derive(Debug, Clone)]
pub struct DepositData {
    pub tx_hash: String,
    pub block_number: u64,
    pub log_index: Option<u64>,
    pub from_address: String,
    pub to_address: String,
    pub amount: i64,
}

/// Withdrawal confirmation data extracted from blockchain events
#[derive(Debug, Clone)]
pub struct WithdrawalConfirmation {
    pub tx_hash: String,
    pub block_number: u64,
    pub from_address: String,
    pub recipient_address: String,
    pub amount: i64,
}

/// Handler for deposit events from the bridge contract
///
/// When a user deposits USDC on L1, the bridge emits a `Deposit` event.
/// This handler captures those events and sends them via channel to be
/// inserted into `inbound_deposits` by the main thread.
#[derive(Debug)]
pub struct DepositHandler {
    tx: Sender<DepositData>,
    processed_count: std::sync::atomic::AtomicU64,
}

impl DepositHandler {
    /// Create a new deposit handler
    pub const fn new(tx: Sender<DepositData>) -> Self {
        Self {
            tx,
            processed_count: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// Get the number of deposits processed
    pub fn processed_count(&self) -> u64 {
        self.processed_count
            .load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl MessageHandler for DepositHandler {
    async fn handle_event(&self, log: &Log) -> Result<bool> {
        // Decode the deposit event
        let deposit = match Deposit::decode_log(&log.inner) {
            Ok(d) => d,
            Err(e) => {
                warn!("Failed to decode deposit event: {}", e);
                return Ok(false);
            }
        };

        let tx_hash = log
            .transaction_hash
            .map(|h| format!("{h:#x}"))
            .unwrap_or_default();
        let block_number = log.block_number.unwrap_or(0);
        let log_index = log.log_index;

        // Convert amount from U256 to i64 (assuming reasonable values)
        let amount: i64 = deposit.amount.try_into().unwrap_or_else(|_| {
            warn!("Deposit amount too large, capping at i64::MAX");
            i64::MAX
        });

        // Validate
        if amount == 0 {
            warn!(tx_hash = %tx_hash, "Deposit with zero amount - skipping");
            return Ok(false);
        }

        info!(
            tx_hash = %tx_hash,
            from = %format!("{:#x}", deposit.from),
            to = %format!("{:#x}", deposit.to),
            amount = amount,
            block = block_number,
            "Processing deposit event"
        );

        // Send via channel
        let data = DepositData {
            tx_hash,
            block_number,
            log_index,
            from_address: format!("{:#x}", deposit.from),
            to_address: format!("{:#x}", deposit.to),
            amount,
        };

        match self.tx.send(data) {
            Ok(_) => {
                self.processed_count
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(true)
            }
            Err(e) => {
                error!("Failed to send deposit via channel: {}", e);
                Ok(false)
            }
        }
    }

    fn event_signature(&self) -> Option<B256> {
        Some(Deposit::SIGNATURE_HASH)
    }

    async fn on_start(&self) -> Result<()> {
        info!("DepositHandler started - watching for bridge deposits");
        Ok(())
    }

    async fn on_stop(&self) -> Result<()> {
        info!(processed = self.processed_count(), "DepositHandler stopped");
        Ok(())
    }
}

/// Handler for withdrawal confirmation events from the bridge contract
///
/// When a withdrawal is processed on L1, the bridge emits a `Withdrawal` event.
/// This handler captures those events and sends them via channel to update
/// the corresponding `outbound_withdrawals` record to 'confirmed' status.
#[derive(Debug)]
pub struct WithdrawalHandler {
    tx: Sender<WithdrawalConfirmation>,
    processed_count: std::sync::atomic::AtomicU64,
}

impl WithdrawalHandler {
    /// Create a new withdrawal handler
    pub const fn new(tx: Sender<WithdrawalConfirmation>) -> Self {
        Self {
            tx,
            processed_count: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// Get the number of withdrawals confirmed
    pub fn processed_count(&self) -> u64 {
        self.processed_count
            .load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl MessageHandler for WithdrawalHandler {
    async fn handle_event(&self, log: &Log) -> Result<bool> {
        // Decode the withdrawal event
        let withdrawal = match Withdrawal::decode_log(&log.inner) {
            Ok(w) => w,
            Err(e) => {
                warn!("Failed to decode withdrawal event: {}", e);
                return Ok(false);
            }
        };

        let tx_hash = log
            .transaction_hash
            .map(|h| format!("{h:#x}"))
            .unwrap_or_default();
        let block_number = log.block_number.unwrap_or(0);

        // Convert amount from U256 to i64
        let amount: i64 = withdrawal.amount.try_into().unwrap_or_else(|_| {
            warn!("Withdrawal amount too large, capping at i64::MAX");
            i64::MAX
        });

        info!(
            tx_hash = %tx_hash,
            from = %format!("{:#x}", withdrawal.from),
            recipient = %format!("{:#x}", withdrawal.recipient),
            amount = amount,
            block = block_number,
            "Processing withdrawal confirmation"
        );

        // Send via channel
        let data = WithdrawalConfirmation {
            tx_hash,
            block_number,
            from_address: format!("{:#x}", withdrawal.from),
            recipient_address: format!("{:#x}", withdrawal.recipient),
            amount,
        };

        match self.tx.send(data) {
            Ok(_) => {
                self.processed_count
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(true)
            }
            Err(e) => {
                error!("Failed to send withdrawal confirmation via channel: {}", e);
                Ok(false)
            }
        }
    }

    fn event_signature(&self) -> Option<B256> {
        Some(Withdrawal::SIGNATURE_HASH)
    }

    async fn on_start(&self) -> Result<()> {
        info!("WithdrawalHandler started - watching for withdrawal confirmations");
        Ok(())
    }

    async fn on_stop(&self) -> Result<()> {
        info!(
            processed = self.processed_count(),
            "WithdrawalHandler stopped"
        );
        Ok(())
    }
}

/// Insert a deposit record into the database
///
/// Called by the main thread after receiving deposit data from the handler.
pub fn insert_deposit(conn: &rusqlite::Connection, deposit: &DepositData) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO inbound_deposits
         (tx_hash, from_address, to_address, amount, block_number, log_index)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![
            deposit.tx_hash,
            deposit.from_address,
            deposit.to_address,
            deposit.amount,
            deposit.block_number as i64,
            deposit.log_index.map(|i| i as i64),
        ],
    )?;
    Ok(())
}

/// Confirm a withdrawal in the database
///
/// Called by the main thread after receiving confirmation from the handler.
pub fn confirm_withdrawal(
    conn: &rusqlite::Connection,
    confirmation: &WithdrawalConfirmation,
) -> Result<usize> {
    let confirmed_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let rows = conn.execute(
        "UPDATE outbound_withdrawals
         SET status = 'confirmed', l1_tx_hash = ?1, confirmed_at = ?2
         WHERE destination_address = ?3
           AND amount = ?4
           AND status IN ('pending', 'submitted')
         ORDER BY created_at ASC
         LIMIT 1",
        rusqlite::params![
            confirmation.tx_hash,
            confirmed_at,
            confirmation.recipient_address,
            confirmation.amount,
        ],
    )?;

    Ok(rows)
}

/// Composite handler that processes both Deposit and Withdrawal events
///
/// Since `ChainMonitor` accepts a single handler, this composite dispatches
/// to both deposit and withdrawal processing logic. It does not filter by
/// event signature, allowing all events from the bridge contract to be received.
#[derive(Debug)]
pub struct BridgeEventHandler {
    deposit_tx: Sender<DepositData>,
    withdrawal_tx: Sender<WithdrawalConfirmation>,
    deposit_count: std::sync::atomic::AtomicU64,
    withdrawal_count: std::sync::atomic::AtomicU64,
}

impl BridgeEventHandler {
    /// Create a new bridge event handler
    pub const fn new(
        deposit_tx: Sender<DepositData>,
        withdrawal_tx: Sender<WithdrawalConfirmation>,
    ) -> Self {
        Self {
            deposit_tx,
            withdrawal_tx,
            deposit_count: std::sync::atomic::AtomicU64::new(0),
            withdrawal_count: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// Get the number of deposits processed
    pub fn deposit_count(&self) -> u64 {
        self.deposit_count.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Get the number of withdrawals processed
    pub fn withdrawal_count(&self) -> u64 {
        self.withdrawal_count
            .load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl MessageHandler for BridgeEventHandler {
    async fn handle_event(&self, log: &Log) -> Result<bool> {
        // Try to decode as Deposit first
        if let Ok(deposit) = Deposit::decode_log(&log.inner) {
            let tx_hash = log
                .transaction_hash
                .map(|h| format!("{h:#x}"))
                .unwrap_or_default();
            let block_number = log.block_number.unwrap_or(0);
            let log_index = log.log_index;

            let amount: i64 = deposit.amount.try_into().unwrap_or_else(|_| {
                warn!("Deposit amount too large, capping at i64::MAX");
                i64::MAX
            });

            if amount == 0 {
                warn!(tx_hash = %tx_hash, "Deposit with zero amount - skipping");
                return Ok(false);
            }

            info!(
                tx_hash = %tx_hash,
                from = %format!("{:#x}", deposit.from),
                to = %format!("{:#x}", deposit.to),
                amount = amount,
                block = block_number,
                "Processing deposit event"
            );

            let data = DepositData {
                tx_hash,
                block_number,
                log_index,
                from_address: format!("{:#x}", deposit.from),
                to_address: format!("{:#x}", deposit.to),
                amount,
            };

            if self.deposit_tx.send(data).is_ok() {
                self.deposit_count
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                return Ok(true);
            }
            return Ok(false);
        }

        // Try to decode as Withdrawal
        if let Ok(withdrawal) = Withdrawal::decode_log(&log.inner) {
            let tx_hash = log
                .transaction_hash
                .map(|h| format!("{h:#x}"))
                .unwrap_or_default();
            let block_number = log.block_number.unwrap_or(0);

            let amount: i64 = withdrawal.amount.try_into().unwrap_or_else(|_| {
                warn!("Withdrawal amount too large, capping at i64::MAX");
                i64::MAX
            });

            info!(
                tx_hash = %tx_hash,
                from = %format!("{:#x}", withdrawal.from),
                recipient = %format!("{:#x}", withdrawal.recipient),
                amount = amount,
                block = block_number,
                "Processing withdrawal confirmation"
            );

            let data = WithdrawalConfirmation {
                tx_hash,
                block_number,
                from_address: format!("{:#x}", withdrawal.from),
                recipient_address: format!("{:#x}", withdrawal.recipient),
                amount,
            };

            if self.withdrawal_tx.send(data).is_ok() {
                self.withdrawal_count
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                return Ok(true);
            }
            return Ok(false);
        }

        // Event didn't match either type
        warn!("Received unknown event type from bridge contract");
        Ok(false)
    }

    fn event_signature(&self) -> Option<B256> {
        // Return None to receive all events from the contract
        None
    }

    async fn on_start(&self) -> Result<()> {
        info!("BridgeEventHandler started - watching for deposits and withdrawals");
        Ok(())
    }

    async fn on_stop(&self) -> Result<()> {
        info!(
            deposits = self.deposit_count(),
            withdrawals = self.withdrawal_count(),
            "BridgeEventHandler stopped"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossbeam_channel::unbounded;

    #[test]
    fn test_deposit_handler_creation() {
        let (tx, _rx) = unbounded();
        let handler = DepositHandler::new(tx);
        assert!(handler.event_signature().is_some());
        assert_eq!(handler.processed_count(), 0);
    }

    #[test]
    fn test_withdrawal_handler_creation() {
        let (tx, _rx) = unbounded();
        let handler = WithdrawalHandler::new(tx);
        assert!(handler.event_signature().is_some());
        assert_eq!(handler.processed_count(), 0);
    }

    #[test]
    fn test_bridge_event_handler_creation() {
        let (deposit_tx, _deposit_rx) = unbounded();
        let (withdrawal_tx, _withdrawal_rx) = unbounded();
        let handler = BridgeEventHandler::new(deposit_tx, withdrawal_tx);
        assert!(handler.event_signature().is_none()); // Receives all events
        assert_eq!(handler.deposit_count(), 0);
        assert_eq!(handler.withdrawal_count(), 0);
    }
}
