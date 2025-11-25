//! `SyndDB` Client Library - Lightweight `SQLite` Session Extension Wrapper
//!
//! This library provides a minimal integration layer for applications to send
//! changesets to the `SyndDB` sequencer. It runs in the application's TEE and
//! does NOT contain any signing keys.
//!
//! # Usage
//!
//! ```rust,no_run
//! use rusqlite::Connection;
//! use synddb_client::SyndDB;
//!
//! // Note: Connection must have 'static lifetime
//! let conn = Box::leak(Box::new(Connection::open("app.db")?));
//! let _synddb = SyndDB::attach(conn, "https://sequencer:8433")?;
//!
//! // Use SQLite normally - changesets are automatically captured and published every 1 second
//! conn.execute("INSERT INTO trades VALUES (?1, ?2)", rusqlite::params![1, 100])?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use anyhow::Result;
use crossbeam_channel::{bounded, Sender};
use rusqlite::Connection;
use std::thread;
use tracing::{debug, info, warn};

pub mod attestation;
pub mod config;
pub mod recovery;
pub mod retry;
pub mod sender;
pub mod session;
pub mod snapshot_sender;

#[cfg(feature = "ffi")]
pub mod ffi;

#[cfg(feature = "chain-monitor")]
pub mod chain_handler;

pub mod chain_monitor_integration;
use chain_monitor_integration::ChainMonitorHandle;

pub use attestation::{is_confidential_space, AttestationClient, TokenType};
pub use config::Config;
use recovery::FailedBatchRecovery;
use sender::ChangesetSender;
use session::SessionMonitor;
pub use session::Snapshot;
use snapshot_sender::SnapshotSender;

/// Main handle to `SyndDB` client
///
/// Attaches to a `SQLite` connection and automatically captures changesets.
/// Dropping this handle will stop changeset capture and publish pending data.
#[derive(Debug)]
pub struct SyndDB {
    /// Session monitor for capturing changesets (includes publish thread)
    monitor: Option<SessionMonitor>,
    /// Channel to send shutdown signal to changeset sender
    changeset_shutdown_tx: Sender<()>,
    /// Channel to send shutdown signal to snapshot sender
    snapshot_shutdown_tx: Option<Sender<()>>,
    /// Handle to background changeset sender thread
    changeset_handle: Option<thread::JoinHandle<()>>,
    /// Handle to background snapshot sender thread
    snapshot_handle: Option<thread::JoinHandle<()>>,
    /// Optional recovery storage for failed batches
    recovery: Option<std::sync::Arc<FailedBatchRecovery>>,
    /// Optional chain monitor handle (enabled with `chain-monitor` feature)
    chain_monitor: Option<ChainMonitorHandle>,
}

/// Statistics about failed batches in recovery storage
#[derive(Debug, Clone, Copy)]
pub struct RecoveryStats {
    /// Number of failed changesets waiting to be retried
    pub failed_changesets: usize,
    /// Number of failed snapshots waiting to be retried
    pub failed_snapshots: usize,
}

impl SyndDB {
    /// Attach to an existing `SQLite` connection
    ///
    /// This will:
    /// 1. Enable `SQLite` Session Extension on the connection
    /// 2. Register update hooks to detect changes
    /// 3. Start a background thread to send changesets to sequencer
    ///
    /// # Arguments
    ///
    /// * `conn` - `SQLite` connection to monitor (must have `'static` lifetime)
    /// * `sequencer_url` - URL of the sequencer TEE (e.g. "<https://sequencer:8433>")
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use rusqlite::Connection;
    /// # use synddb_client::SyndDB;
    /// // Note: Connection must have 'static lifetime
    /// let conn = Box::leak(Box::new(Connection::open("app.db")?));
    /// let synddb = SyndDB::attach(conn, "https://sequencer:8433")?;
    ///
    /// // Perform database operations...
    /// conn.execute("INSERT INTO users VALUES (?1, ?2)", rusqlite::params![1, "Alice"])?;
    ///
    /// // Changesets are automatically published every 1 second
    /// // You can also manually publish for critical transactions:
    /// synddb.publish()?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn attach(conn: &'static Connection, sequencer_url: impl Into<String>) -> Result<Self> {
        Self::attach_with_config(
            conn,
            Config {
                sequencer_url: sequencer_url.into(),
                ..Default::default()
            },
        )
    }

    /// Attach with custom configuration
    pub fn attach_with_config(conn: &'static Connection, config: Config) -> Result<Self> {
        info!("Attaching SyndDB client to SQLite connection");
        info!("Sequencer URL: {}", config.sequencer_url);
        info!("Publish interval: {:?}", config.publish_interval);

        // Create channels for communication
        let (changeset_tx, changeset_rx) = bounded(config.buffer_size);
        let (changeset_shutdown_tx, changeset_shutdown_rx) = bounded(1);

        // Create snapshot channel if automatic snapshots are enabled
        let snapshot_channel = (config.snapshot_interval > 0).then(|| bounded(10)); // Buffer up to 10 snapshots

        // Start session monitor (includes automatic publish thread)
        let monitor = SessionMonitor::new(
            conn,
            changeset_tx,
            config.publish_interval,
            config.snapshot_interval,
            snapshot_channel.as_ref().map(|(tx, _)| tx).cloned(),
        )?;
        monitor.start(conn)?;

        // Create shared recovery storage for failed batches
        // This is optional - if None, failed batches are dropped
        let recovery = if config.enable_recovery {
            // Use a stable path based on the sequencer URL to persist across restarts
            // Hash the sequencer URL to create a stable but unique identifier
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};

            let mut hasher = DefaultHasher::new();
            config.sequencer_url.hash(&mut hasher);
            let url_hash = hasher.finish();

            let temp_dir = std::env::temp_dir();
            let db_name = format!("synddb_recovery_{:x}.db", url_hash);
            let recovery_path = temp_dir.join(db_name);

            match FailedBatchRecovery::new(recovery_path) {
                Ok(r) => Some(std::sync::Arc::new(r)),
                Err(e) => {
                    warn!(
                        "Failed to initialize recovery storage: {}. Continuing without recovery.",
                        e
                    );
                    None
                }
            }
        } else {
            None
        };

        // Create attestation client if enabled (only works in GCP Confidential Space)
        let attestation_client = if config.enable_attestation {
            match AttestationClient::new(&config.sequencer_url, config.attestation_token_type) {
                Ok(client) => {
                    info!(
                        "Attestation enabled (type: {:?})",
                        config.attestation_token_type
                    );
                    Some(client)
                }
                Err(e) => {
                    warn!(
                        "Attestation requested but unavailable: {}. Continuing without attestation.",
                        e
                    );
                    None
                }
            }
        } else {
            info!("Attestation disabled");
            None
        };

        // Start snapshot sender thread if enabled
        let (snapshot_shutdown_tx, snapshot_handle) = snapshot_channel
            .map(|(_, snapshot_rx)| {
                let (shutdown_tx, shutdown_rx) = bounded(1);
                let cfg = config.clone();
                let rec = recovery.clone();
                let att = attestation_client.clone();

                let handle = thread::spawn(move || {
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .expect("Failed to create tokio runtime for snapshot sender")
                        .block_on(async {
                            SnapshotSender::new(cfg, rec, att)
                                .run(snapshot_rx, shutdown_rx)
                                .await
                        });
                });

                (Some(shutdown_tx), Some(handle))
            })
            .unwrap_or_default();

        // Start background changeset sender thread
        let changeset_handle = thread::spawn({
            let recovery_clone = recovery.clone();
            let config_clone = config.clone();
            move || {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("Failed to create tokio runtime for changeset sender")
                    .block_on(async {
                        ChangesetSender::new(config_clone, recovery_clone, attestation_client)
                            .run(changeset_rx, changeset_shutdown_rx)
                            .await
                    });
            }
        });

        // Start chain monitor if configured
        let chain_monitor = config.chain_monitor.and_then(|chain_config| {
            match ChainMonitorHandle::new(chain_config, conn) {
                Ok(handle) => {
                    info!("Chain monitor started successfully");
                    Some(handle)
                }
                Err(e) => {
                    warn!(
                        "Failed to start chain monitor: {}. Continuing without it.",
                        e
                    );
                    None
                }
            }
        });

        info!("SyndDB client attached successfully");

        Ok(Self {
            monitor: Some(monitor),
            changeset_shutdown_tx,
            snapshot_shutdown_tx,
            changeset_handle: Some(changeset_handle),
            snapshot_handle,
            recovery,
            chain_monitor,
        })
    }

    /// Publish all pending changesets to the sequencer
    ///
    /// This is called automatically every `publish_interval` (default 1 second),
    /// but can also be called manually to publish immediately (e.g., after critical transactions).
    pub fn publish(&self) -> Result<()> {
        self.monitor
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Monitor already shut down"))?
            .publish()
    }

    /// Create a complete snapshot of the database
    ///
    /// This captures the full current state of the database as a portable `SQLite` file.
    /// The snapshot includes the current sequence number, so replicas can know which
    /// changesets to apply after restoring from this snapshot.
    ///
    /// # Use Case
    ///
    /// Snapshots are used when new replicas join the network and need to sync from
    /// the current state rather than replaying all changesets from genesis.
    ///
    /// # Returns
    ///
    /// A `Snapshot` containing:
    /// - Complete database file as bytes (cross-platform portable)
    /// - Current sequence number (changesets with seq >= this apply after snapshot)
    /// - Timestamp of snapshot creation
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use rusqlite::Connection;
    /// # use synddb_client::SyndDB;
    /// # let conn = Box::leak(Box::new(Connection::open("app.db").unwrap()));
    /// # let synddb = SyndDB::attach(conn, "http://localhost:8433").unwrap();
    /// // Create snapshot for new replicas
    /// let snapshot = synddb.snapshot()?;
    ///
    /// println!("Snapshot size: {} bytes", snapshot.data.len());
    /// println!("Snapshot at sequence: {}", snapshot.sequence);
    ///
    /// // Replicas would:
    /// // 1. Restore from snapshot.data
    /// // 2. Apply changesets with sequence >= snapshot.sequence
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn snapshot(&self) -> Result<Snapshot> {
        self.monitor
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Monitor already shut down"))?
            .snapshot()
    }

    /// Process pending deposits from the blockchain
    ///
    /// This method should be called periodically to process incoming deposit events
    /// from the blockchain and insert them into the local database.
    ///
    /// # Returns
    ///
    /// The number of deposits processed, or an error if the chain monitor is not enabled
    ///
    /// # Note
    ///
    /// This method requires the "chain-monitor" feature to be enabled at compile time.
    /// If the feature is not enabled, this will return an error.
    pub fn process_deposits(&self) -> Result<usize> {
        self.chain_monitor
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Chain monitor not enabled or failed to start"))?
            .process_deposits()
    }

    /// Get statistics about failed batches in recovery storage
    ///
    /// Returns the number of failed changesets and snapshots waiting to be retried.
    /// Returns `None` if recovery is disabled.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use rusqlite::Connection;
    /// # use synddb_client::SyndDB;
    /// # let conn = Box::leak(Box::new(Connection::open("app.db").unwrap()));
    /// # let synddb = SyndDB::attach(conn, "http://localhost:8433").unwrap();
    /// if let Some(stats) = synddb.recovery_stats()? {
    ///     println!("Failed changesets: {}", stats.failed_changesets);
    ///     println!("Failed snapshots: {}", stats.failed_snapshots);
    /// }
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn recovery_stats(&self) -> Result<Option<RecoveryStats>> {
        match &self.recovery {
            Some(recovery) => {
                let (failed_changesets, failed_snapshots) = recovery.get_failed_counts()?;
                Ok(Some(RecoveryStats {
                    failed_changesets,
                    failed_snapshots,
                }))
            }
            None => Ok(None),
        }
    }

    /// Gracefully shutdown the client, publishing any pending changesets and snapshots
    pub fn shutdown(mut self) -> Result<()> {
        info!("Shutting down SyndDB client");

        // Send shutdown signals
        let _ = self.changeset_shutdown_tx.send(());
        if let Some(ref tx) = self.snapshot_shutdown_tx {
            let _ = tx.send(());
        }

        // Wait for changeset sender thread to finish
        if let Some(handle) = self.changeset_handle.take() {
            handle.join().expect("Changeset sender thread panicked");
        }

        // Wait for snapshot sender thread to finish
        if let Some(handle) = self.snapshot_handle.take() {
            handle.join().expect("Snapshot sender thread panicked");
        }

        // Note: Chain monitor thread runs indefinitely and will be aborted on Drop
        // This is expected behavior as the monitor should run as long as the client is active

        info!("SyndDB client shut down successfully");
        Ok(())
    }
}

impl Drop for SyndDB {
    fn drop(&mut self) {
        debug!("Dropping SyndDB handle");

        // First, drop the monitor which will stop the publish thread
        // This ensures no more changesets or snapshots are generated
        if let Some(monitor) = self.monitor.take() {
            drop(monitor);
        }

        // Then send shutdown signals to sender threads
        let _ = self.changeset_shutdown_tx.send(());
        if let Some(ref tx) = self.snapshot_shutdown_tx {
            let _ = tx.send(());
        }

        // Wait for changeset sender thread
        if let Some(handle) = self.changeset_handle.take() {
            if let Err(e) = handle.join() {
                warn!("Changeset sender thread panicked during drop: {:?}", e);
            }
        }

        // Wait for snapshot sender thread
        if let Some(handle) = self.snapshot_handle.take() {
            if let Err(e) = handle.join() {
                warn!("Snapshot sender thread panicked during drop: {:?}", e);
            }
        }

        // Drop chain monitor if present
        // Note: Chain monitor thread runs indefinitely and will be terminated on process exit
        if self.chain_monitor.is_some() {
            debug!("Chain monitor will be terminated on process exit");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_attach() {
        let conn = Box::leak(Box::new(Connection::open_in_memory().unwrap()));
        conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)", [])
            .unwrap();

        let _synddb = SyndDB::attach(conn, "http://localhost:8433").unwrap();

        conn.execute("INSERT INTO test (id, value) VALUES (1, 'test')", [])
            .unwrap();

        // Wait a moment for automatic publish
        thread::sleep(std::time::Duration::from_secs(2));
    }
}
