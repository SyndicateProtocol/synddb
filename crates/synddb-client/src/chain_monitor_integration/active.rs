//! Active chain monitor implementation (enabled with "chain-monitor" feature)

use crate::chain_handler::{DepositData, DepositHandler};
use anyhow::Result;
use crossbeam_channel::{bounded, Receiver};
use rusqlite::Connection;
use std::thread::{self, JoinHandle};
use synddb_chain_monitor::{config::ChainMonitorConfig, monitor::ChainMonitor};
use tokio::sync::watch;
use tracing::{debug, error, info, warn};

/// Handle for an active chain monitor
///
/// This manages a background thread that monitors blockchain events and
/// processes deposits into a local `SQLite` database.
pub struct ChainMonitorHandle {
    /// Background thread handle (Option to allow taking in Drop)
    handle: Option<JoinHandle<()>>,
    /// Shutdown signal sender
    shutdown_tx: watch::Sender<bool>,
    /// Channel receiving deposit data from chain monitor
    deposit_rx: Receiver<DepositData>,
    /// `SQLite` connection for inserting deposits
    conn: &'static Connection,
    /// Name of the table to insert deposits into
    table_name: String,
}

impl std::fmt::Debug for ChainMonitorHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChainMonitorHandle")
            .field("table_name", &self.table_name)
            .field("running", &self.handle.is_some())
            .finish()
    }
}

impl ChainMonitorHandle {
    /// Create and start a new chain monitor
    ///
    /// This spawns a background thread that:
    /// 1. Connects to the blockchain via WebSocket/RPC
    /// 2. Monitors the specified contract for deposit events
    /// 3. Sends deposit data through a channel for processing
    pub fn new(config: ChainMonitorConfig, conn: &'static Connection) -> Result<Self> {
        info!(
            "Starting chain monitor for contract {}",
            format!("{:#x}", config.contract_address)
        );

        // Extract deposit table name from config
        let table_name = "deposits".to_string(); // Default table name

        // Create channel for deposit data
        let (deposit_tx, deposit_rx) = bounded::<DepositData>(100);

        // Create shutdown channel
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        // Start chain monitor thread
        let handle = thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("Failed to create tokio runtime for chain monitor")
                .block_on(async {
                    // Create deposit handler
                    let handler = std::sync::Arc::new(DepositHandler::new(deposit_tx));

                    // Create and run chain monitor
                    match ChainMonitor::new(config, handler).await {
                        Ok(mut monitor) => {
                            info!("Chain monitor started successfully");
                            if let Err(e) = monitor.run(shutdown_rx).await {
                                error!("Chain monitor error: {}", e);
                            }
                            info!("Chain monitor stopped");
                        }
                        Err(e) => {
                            error!("Failed to start chain monitor: {}", e);
                        }
                    }
                });
        });

        // Ensure deposits table exists
        let create_table_sql = format!(
            "CREATE TABLE IF NOT EXISTS {} (
                tx_hash TEXT PRIMARY KEY,
                block_number INTEGER NOT NULL,
                log_index INTEGER,
                from_address TEXT NOT NULL,
                to_address TEXT NOT NULL,
                amount TEXT NOT NULL,
                data BLOB,
                processed_at INTEGER NOT NULL
            )",
            table_name
        );
        conn.execute(&create_table_sql, [])?;
        info!("Deposits table '{}' ready", table_name);

        Ok(Self {
            handle: Some(handle),
            shutdown_tx,
            deposit_rx,
            conn,
            table_name,
        })
    }

    /// Process pending deposits from the blockchain
    ///
    /// This method processes all pending deposit events that have been received
    /// from the blockchain and inserts them into the local database.
    ///
    /// # Returns
    ///
    /// The number of deposits processed
    pub fn process_deposits(&self) -> Result<usize> {
        let mut count = 0;

        // Process all pending deposits (non-blocking)
        while let Ok(deposit) = self.deposit_rx.try_recv() {
            let insert_sql = format!(
                "INSERT OR IGNORE INTO {} (tx_hash, block_number, log_index, from_address, to_address, amount, data, processed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                self.table_name
            );

            let processed_at = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64;

            self.conn.execute(
                &insert_sql,
                rusqlite::params![
                    deposit.tx_hash,
                    deposit.block_number as i64,
                    deposit.log_index.map(|i| i as i64),
                    deposit.from,
                    deposit.to,
                    deposit.amount,
                    deposit.data,
                    processed_at,
                ],
            )?;
            count += 1;
        }

        if count > 0 {
            debug!("Processed {} deposits", count);
        }

        Ok(count)
    }

    /// Gracefully shutdown the chain monitor
    ///
    /// This signals the monitor to stop and waits for the background thread to exit.
    /// Called automatically on Drop, but can be called explicitly for more control.
    pub fn shutdown(&mut self) {
        // Signal shutdown
        if let Err(e) = self.shutdown_tx.send(true) {
            warn!("Failed to send shutdown signal: {}", e);
        }

        // Wait for thread to finish
        if let Some(handle) = self.handle.take() {
            debug!("Waiting for chain monitor thread to exit...");
            match handle.join() {
                Ok(()) => info!("Chain monitor thread exited cleanly"),
                Err(e) => error!("Chain monitor thread panicked: {:?}", e),
            }
        }
    }
}

impl Drop for ChainMonitorHandle {
    fn drop(&mut self) {
        debug!("ChainMonitorHandle dropping, initiating shutdown");
        self.shutdown();
    }
}
