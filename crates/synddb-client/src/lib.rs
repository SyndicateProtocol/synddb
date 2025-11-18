//! SyndDB Client Library - Lightweight SQLite Session Extension Wrapper
//!
//! This library provides a minimal integration layer for applications to send
//! changesets to the SyndDB sequencer. It runs in the application's TEE and
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
//! // Use SQLite normally - changesets are automatically captured and sent every 1 second
//! conn.execute("INSERT INTO trades VALUES (?1, ?2)", rusqlite::params![1, 100])?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use anyhow::Result;
use crossbeam_channel::{bounded, Sender};
use rusqlite::Connection;
use std::thread;
use tracing::{debug, info, warn};

mod config;
mod sender;
mod session;

pub use config::Config;
use sender::ChangesetSender;
use session::SessionMonitor;
pub use session::Snapshot;

/// Main handle to SyndDB client
///
/// Attaches to a SQLite connection and automatically captures changesets.
/// Dropping this handle will stop changeset capture and flush pending data.
pub struct SyndDB {
    /// Session monitor for capturing changesets (includes flush thread)
    monitor: Option<SessionMonitor>,
    /// Channel to send shutdown signal
    shutdown_tx: Sender<()>,
    /// Handle to background sender thread
    join_handle: Option<thread::JoinHandle<()>>,
}

impl SyndDB {
    /// Attach to an existing SQLite connection
    ///
    /// This will:
    /// 1. Enable SQLite Session Extension on the connection
    /// 2. Register update hooks to detect changes
    /// 3. Start a background thread to send changesets to sequencer
    ///
    /// # Arguments
    ///
    /// * `conn` - SQLite connection to monitor (must have 'static lifetime)
    /// * `sequencer_url` - URL of the sequencer TEE (e.g. "https://sequencer:8433")
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
    /// // Changesets are automatically flushed every 1 second
    /// // You can also manually flush for critical transactions:
    /// synddb.flush()?;
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
        info!("Flush interval: {:?}", config.flush_interval);

        // Create channels for communication
        let (changeset_tx, changeset_rx) = bounded(config.buffer_size);
        let (shutdown_tx, shutdown_rx) = bounded(1);

        // Start session monitor (includes automatic flush thread)
        let monitor = SessionMonitor::new(conn, changeset_tx.clone(), config.flush_interval)?;
        monitor.start(conn)?;

        // Start background sender thread
        let sender_config = config.clone();
        let join_handle = thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("Failed to create tokio runtime");

            rt.block_on(async {
                let sender = ChangesetSender::new(sender_config);
                sender.run(changeset_rx, shutdown_rx).await
            });
        });

        info!("SyndDB client attached successfully");

        Ok(Self {
            monitor: Some(monitor),
            shutdown_tx,
            join_handle: Some(join_handle),
        })
    }

    /// Flush all pending changesets to the sequencer
    ///
    /// This is called automatically every flush_interval (default 1 second),
    /// but can also be called manually to flush immediately (e.g., after critical transactions).
    pub fn flush(&self) -> Result<()> {
        self.monitor
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Monitor already shut down"))?
            .flush()
    }

    /// Create a complete snapshot of the database
    ///
    /// This captures the full current state of the database as a portable SQLite file.
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

    /// Gracefully shutdown the client, flushing any pending changesets
    pub fn shutdown(mut self) -> Result<()> {
        info!("Shutting down SyndDB client");

        // Send shutdown signal
        let _ = self.shutdown_tx.send(());

        // Wait for background thread to finish
        if let Some(handle) = self.join_handle.take() {
            handle.join().expect("Background thread panicked");
        }

        info!("SyndDB client shut down successfully");
        Ok(())
    }
}

impl Drop for SyndDB {
    fn drop(&mut self) {
        debug!("Dropping SyndDB handle");

        // First, drop the monitor which will stop the flush thread
        // This ensures no more changesets are generated
        if let Some(monitor) = self.monitor.take() {
            drop(monitor);
        }

        // Then send shutdown signal to sender thread
        let _ = self.shutdown_tx.send(());

        // Wait for background sender thread
        if let Some(handle) = self.join_handle.take() {
            if let Err(e) = handle.join() {
                warn!("Background thread panicked during drop: {:?}", e);
            }
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

        // Wait a moment for automatic flush
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
}
