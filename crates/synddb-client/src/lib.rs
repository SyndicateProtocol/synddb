//! `SyndDB` Client Library - `SQLite` Session Extension Wrapper
//!
//! This library captures `SQLite` changesets and sends them to the `SyndDB` sequencer.
//! It runs in the application's TEE and does NOT contain any signing keys.
//!
//! # Usage
//!
//! ```rust,no_run
//! use rusqlite::Connection;
//! use synddb_client::SyndDB;
//!
//! // Connection requires 'static lifetime (see "Why `'static` lifetime?" section below)
//! let conn = Box::leak(Box::new(Connection::open("app.db")?));
//! let synddb = SyndDB::attach(conn, "http://sequencer:8433")?;
//!
//! // Use SQLite normally - changesets are captured automatically
//! conn.execute("CREATE TABLE trades (id INTEGER, amount INTEGER)", [])?;
//! conn.execute("INSERT INTO trades VALUES (?1, ?2)", rusqlite::params![1, 100])?;
//!
//! // For transactions, use unchecked_transaction() instead of transaction()
//! let tx = conn.unchecked_transaction()?;
//! tx.execute("INSERT INTO trades VALUES (?1, ?2)", rusqlite::params![2, 200])?;
//! tx.commit()?;
//!
//! // Publish changesets to sequencer
//! synddb.publish()?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! # Publishing
//!
//! You must call [`SyndDB::publish()`] to send captured changesets to the sequencer:
//! - After committing a transaction
//! - After a batch of related operations
//! - Periodically in long-running applications
//!
//! Changesets are also published when `SyndDB` is dropped (graceful shutdown).
//!
//! # Transactions
//!
//! Use [`Connection::unchecked_transaction()`] instead of [`Connection::transaction()`]
//! because `SyndDB` holds an immutable borrow of the connection for the Session Extension.
//!
//! # Thread Safety
//!
//! The Session Extension is only accessed from the main thread. Background threads
//! handle network I/O but only receive `Vec<u8>` bytes through channels.
//!
//! # Why `'static` lifetime?
//!
//! `SyndDB` requires `&'static Connection` because the `SQLite` Session Extension is stored
//! in thread-local storage, which requires `'static` bounds. We use `Box::leak` to satisfy
//! this requirement. This means the Connection is intentionally never dropped - cleanup
//! happens at process exit. This is acceptable for typical single-connection-per-process
//! usage but means `SQLite`'s `Drop` cleanup (closing file handles, WAL checkpoint) won't run.
//!
//! `SyndDB` itself is dropped normally and performs graceful shutdown (publishing pending
//! changesets, joining background threads).

use anyhow::Result;
use crossbeam_channel::{bounded, Sender};
use rusqlite::Connection;
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    thread,
};
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
    pub fn attach(conn: &'static Connection, sequencer_url: &str) -> Result<Self> {
        let url =
            synddb_shared::parse::parse_url(sequencer_url).map_err(|e| anyhow::anyhow!("{}", e))?;
        Self::attach_with_config(
            conn,
            Config {
                sequencer_url: url,
                ..Default::default()
            },
        )
    }

    /// Attach with custom configuration
    pub fn attach_with_config(conn: &'static Connection, config: Config) -> Result<Self> {
        info!("Attaching SyndDB client to SQLite connection");
        info!("Sequencer URL: {}", config.sequencer_url);

        // Create channels for communication
        let (changeset_tx, changeset_rx) = bounded(config.buffer_size);
        let (changeset_shutdown_tx, changeset_shutdown_rx) = bounded(1);

        // Create snapshot channel if automatic snapshots are enabled
        let snapshot_channel = (config.snapshot_interval > 0).then(|| bounded(10)); // Buffer up to 10 snapshots

        // Start session monitor
        let monitor = SessionMonitor::new(
            conn,
            changeset_tx,
            config.snapshot_interval,
            snapshot_channel.as_ref().map(|(tx, _)| tx).cloned(),
        )?;
        monitor.start(conn)?;

        // Create shared recovery storage for failed batches
        // This is optional - if None, failed batches are dropped
        let recovery = if config.enable_recovery {
            let mut hasher = DefaultHasher::new();
            config.sequencer_url.as_str().hash(&mut hasher);
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

        // Create attestation client unless explicitly disabled (enabled by default for production)
        let attestation_client = if config.disable_attestation {
            info!("Attestation disabled");
            None
        } else {
            match AttestationClient::new(
                config.sequencer_url.as_str(),
                config.attestation_token_type,
            ) {
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
    /// Call this after committing transactions to send changesets to the sequencer.
    /// Also called automatically on `Drop` for graceful shutdown.
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
    use std::default::Default;

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

    #[test]
    fn test_drop_graceful_shutdown() {
        let conn = Box::leak(Box::new(Connection::open_in_memory().unwrap()));
        conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)", [])
            .unwrap();

        let synddb = SyndDB::attach(conn, "http://localhost:8433").unwrap();

        // Insert some data
        conn.execute("INSERT INTO test (id, value) VALUES (1, 'test')", [])
            .unwrap();

        // Drop should gracefully shut down all threads without panicking
        drop(synddb);
    }

    #[test]
    fn test_drop_with_pending_changesets() {
        let conn = Box::leak(Box::new(Connection::open_in_memory().unwrap()));
        conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)", [])
            .unwrap();

        let synddb = SyndDB::attach(conn, "http://localhost:8433").unwrap();

        // Insert multiple rows to create pending changesets
        for i in 0..10 {
            conn.execute(
                "INSERT INTO test (id, value) VALUES (?1, ?2)",
                rusqlite::params![i, format!("test{}", i)],
            )
            .unwrap();
        }

        // Drop should handle pending changesets gracefully
        drop(synddb);
    }

    #[test]
    fn test_explicit_shutdown() {
        let conn = Box::leak(Box::new(Connection::open_in_memory().unwrap()));
        conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)", [])
            .unwrap();

        let synddb = SyndDB::attach(conn, "http://localhost:8433").unwrap();

        conn.execute("INSERT INTO test (id, value) VALUES (1, 'test')", [])
            .unwrap();

        // Explicit shutdown should work without error
        synddb.shutdown().unwrap();
    }

    #[test]
    fn test_concurrent_transactions() {
        // This test simulates the orderbook benchmark usage pattern
        // where transactions are run repeatedly while SyndDB is publishing
        let conn = Box::leak(Box::new(Connection::open_in_memory().unwrap()));
        conn.execute(
            "CREATE TABLE orders (id INTEGER PRIMARY KEY, user_id INTEGER, amount INTEGER)",
            [],
        )
        .unwrap();

        let _synddb = SyndDB::attach(conn, "http://localhost:8433").unwrap();

        eprintln!("Starting transaction loop...");

        // Run multiple transaction batches, similar to orderbook benchmark
        for batch in 0..10 {
            eprintln!("Batch {}: starting transaction", batch);

            // Use unchecked_transaction like the benchmark does
            let tx = conn.unchecked_transaction().unwrap();

            for i in 0..10 {
                tx.execute(
                    "INSERT INTO orders (user_id, amount) VALUES (?1, ?2)",
                    rusqlite::params![batch * 10 + i, 1000],
                )
                .unwrap();
            }

            eprintln!("Batch {}: committing", batch);
            tx.commit().unwrap();
            eprintln!("Batch {}: committed", batch);

            // Small delay between batches to allow publish thread to run
            thread::sleep(std::time::Duration::from_millis(200));
        }

        eprintln!("All batches complete, checking row count...");

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM orders", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 100);

        eprintln!("Test passed with {} rows", count);
    }

    #[test]
    fn test_with_automatic_snapshots() {
        // Test with automatic snapshot enabled (like Docker config)
        let conn = Box::leak(Box::new(Connection::open_in_memory().unwrap()));
        conn.execute(
            "CREATE TABLE orders (id INTEGER PRIMARY KEY, user_id INTEGER, amount INTEGER)",
            [],
        )
        .unwrap();

        // Configure with automatic snapshots every 10 changesets (low for testing)
        let config = Config {
            sequencer_url: "http://localhost:8433".parse().unwrap(),
            snapshot_interval: 10,
            ..Default::default()
        };

        let _synddb = SyndDB::attach_with_config(conn, config).unwrap();

        eprintln!("Starting with auto-snapshot every 10 changesets...");

        // Run many transactions to trigger automatic snapshots
        for batch in 0..20 {
            eprintln!("Batch {}: starting transaction", batch);

            let tx = conn.unchecked_transaction().unwrap();

            for i in 0..5 {
                tx.execute(
                    "INSERT INTO orders (user_id, amount) VALUES (?1, ?2)",
                    rusqlite::params![batch * 5 + i, 1000],
                )
                .unwrap();
            }

            eprintln!("Batch {}: committing", batch);
            tx.commit().unwrap();
            eprintln!("Batch {}: committed", batch);

            // Small delay to allow publish thread
            thread::sleep(std::time::Duration::from_millis(100));
        }

        eprintln!("All batches complete");

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM orders", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 100);

        eprintln!("Test passed with {} rows", count);
    }
}
