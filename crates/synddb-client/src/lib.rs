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
//! let conn = Connection::open("app.db")?;
//! let _synddb = SyndDB::attach(&conn, "https://sequencer:8433")?;
//!
//! // Use SQLite normally - changesets are automatically captured
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

/// Main handle to SyndDB client
///
/// Attaches to a SQLite connection and automatically captures changesets.
/// Dropping this handle will stop changeset capture and flush pending data.
pub struct SyndDB {
    /// Session monitor for capturing changesets
    monitor: SessionMonitor,
    /// Channel to send shutdown signal
    shutdown_tx: Sender<()>,
    /// Handle to background thread
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
    /// let conn = Connection::open("app.db")?;
    /// let synddb = SyndDB::attach(&conn, "https://sequencer:8433")?;
    ///
    /// // Perform database operations...
    /// conn.execute("INSERT INTO users VALUES (?1, ?2)", rusqlite::params![1, "Alice"])?;
    ///
    /// // Flush changesets to sequencer
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

        // Create channels for communication
        let (changeset_tx, changeset_rx) = bounded(config.buffer_size);
        let (shutdown_tx, shutdown_rx) = bounded(1);

        // Start session monitor
        let monitor = SessionMonitor::new(conn, changeset_tx.clone())?;
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
            monitor,
            shutdown_tx,
            join_handle: Some(join_handle),
        })
    }

    /// Flush all pending changesets to the sequencer
    ///
    /// This should be called after transactions complete to capture
    /// and send changesets to the sequencer.
    pub fn flush(&self) -> Result<()> {
        self.monitor.flush()
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
        // Send shutdown signal (ignore errors if already shut down)
        let _ = self.shutdown_tx.send(());

        // Wait for background thread
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
        let conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)", [])
            .unwrap();

        let _synddb = SyndDB::attach(&conn, "http://localhost:8433").unwrap();

        conn.execute("INSERT INTO test (id, value) VALUES (1, 'test')", [])
            .unwrap();
    }
}
