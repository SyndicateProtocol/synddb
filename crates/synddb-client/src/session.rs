//! SQLite Session Extension integration

use anyhow::{Context, Result};
use crossbeam_channel::Sender;
use rusqlite::backup::Backup;
use rusqlite::hooks::Action;
use rusqlite::session::Session;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime};
use tracing::{debug, error, info, trace};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Changeset {
    /// Raw changeset bytes from SQLite
    pub data: Vec<u8>,
    /// Sequence number (monotonic)
    pub sequence: u64,
    /// Timestamp when captured
    pub timestamp: SystemTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    /// Complete database snapshot as SQLite database file bytes
    pub data: Vec<u8>,
    /// Timestamp when snapshot was captured
    pub timestamp: SystemTime,
    /// Sequence number at the time of snapshot
    /// (changesets with sequence >= this should be applied after snapshot)
    pub sequence: u64,
}

/// Shared state between session monitor and flush thread
struct SessionState {
    session: Session<'static>,
    changeset_tx: Sender<Changeset>,
    sequence: u64,
    conn: &'static Connection,
}

// SAFETY: SQLite sessions are thread-safe when used with a thread-safe connection.
// We're using a 'static Connection and protecting access with a Mutex, so this is safe.
// The Session contains raw pointers that rusqlite doesn't mark as Send, but SQLite's
// session extension is designed to be thread-safe when properly synchronized.
unsafe impl Send for SessionState {}

pub struct SessionMonitor {
    state: Arc<Mutex<SessionState>>,
    flush_shutdown_tx: Sender<()>,
    flush_handle: Option<thread::JoinHandle<()>>,
}

impl SessionMonitor {
    pub fn new(
        conn: &'static Connection,
        changeset_tx: Sender<Changeset>,
        flush_interval: Duration,
    ) -> Result<Self> {
        debug!("Initializing SQLite Session Extension");

        // Create a session attached to the main database
        let mut session = Session::new(conn).context("Failed to create SQLite session")?;

        // Attach to all tables (None means all tables)
        session
            .attach(None::<&str>)
            .context("Failed to attach session to tables")?;

        debug!("Session attached to all tables");

        let state = Arc::new(Mutex::new(SessionState {
            session,
            changeset_tx,
            sequence: 0,
            conn,
        }));

        // Create channel for flush thread shutdown
        let (flush_shutdown_tx, flush_shutdown_rx) = crossbeam_channel::bounded(1);

        // Start periodic flush thread
        let state_clone = Arc::clone(&state);
        let flush_handle = thread::spawn(move || {
            debug!("Flush thread started with interval {:?}", flush_interval);

            loop {
                // Wait for flush interval or shutdown signal
                match flush_shutdown_rx.recv_timeout(flush_interval) {
                    Ok(()) => {
                        debug!("Flush thread received shutdown signal");
                        // Final flush before shutdown
                        if let Err(e) = Self::flush_internal(&state_clone) {
                            error!("Failed to flush on shutdown: {}", e);
                        }
                        break;
                    }
                    Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                        // Periodic flush
                        if let Err(e) = Self::flush_internal(&state_clone) {
                            error!("Failed to flush periodically: {}", e);
                        }
                    }
                    Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                        debug!("Flush thread channel disconnected");
                        break;
                    }
                }
            }

            debug!("Flush thread stopped");
        });

        debug!("SessionMonitor created");

        Ok(Self {
            state,
            flush_shutdown_tx,
            flush_handle: Some(flush_handle),
        })
    }

    pub fn start(&self, conn: &Connection) -> Result<()> {
        debug!("Installing update hook for automatic changeset capture");

        // Install update hook that gets called on INSERT, UPDATE, DELETE
        conn.update_hook(Some(
            |action: Action, _db: &str, table: &str, rowid: i64| {
                trace!(
                    "Update hook: {:?} on table {} rowid {}",
                    action,
                    table,
                    rowid
                );

                // Note: We can't capture changesets in the hook itself because:
                // 1. Hooks are called during transaction
                // 2. Session::changeset() should be called after transaction commits
                //
                // The hook just serves as a signal that changes occurred.
                // We'll rely on periodic flushing (automatic via flush thread).
            },
        ));

        debug!("SessionMonitor started");
        Ok(())
    }

    /// Internal flush implementation used by both manual and automatic flushing
    fn flush_internal(state: &Arc<Mutex<SessionState>>) -> Result<()> {
        trace!("Flushing session changes");

        let mut state = state.lock().unwrap();

        // Get changeset bytes from session using stream API
        let mut changeset_data = Vec::new();
        state
            .session
            .changeset_strm(&mut changeset_data)
            .context("Failed to get changeset from session")?;

        if changeset_data.is_empty() {
            trace!("No changes to flush");
            return Ok(());
        }

        trace!("Captured changeset: {} bytes", changeset_data.len());

        let changeset = Changeset {
            data: changeset_data,
            sequence: state.sequence,
            timestamp: SystemTime::now(),
        };

        state.sequence += 1;

        // Send to background thread
        state
            .changeset_tx
            .send(changeset)
            .context("Failed to send changeset to background thread")?;

        Ok(())
    }

    /// Capture and send all changes since last flush
    ///
    /// This is called automatically by the flush thread, but can also be called
    /// manually to flush immediately (e.g., after critical transactions).
    pub fn flush(&self) -> Result<()> {
        Self::flush_internal(&self.state)
    }

    /// Create a complete snapshot of the database
    ///
    /// This captures the full current state of the database as a portable SQLite file.
    /// The snapshot includes the current sequence number, so replicas can know which
    /// changesets to apply after restoring from this snapshot.
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
    /// // Send to sequencer or new replica
    /// // Replicas restore from snapshot, then apply changesets with seq >= snapshot.sequence
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn snapshot(&self) -> Result<Snapshot> {
        info!("Creating database snapshot");

        let state = self.state.lock().unwrap();

        // Serialize database to bytes using a temporary file
        // (We can't serialize in-memory db directly, so we use a temp file)
        let snapshot_data = {
            let temp_path =
                std::env::temp_dir().join(format!("synddb_snapshot_{}.db", uuid::Uuid::new_v4()));

            // Backup source to temporary file
            {
                let mut file_conn = Connection::open(&temp_path)
                    .context("Failed to create temporary snapshot file")?;

                let backup = Backup::new(state.conn, &mut file_conn)
                    .context("Failed to initialize backup")?;

                backup
                    .run_to_completion(5, std::time::Duration::from_millis(250), None)
                    .context("Failed to complete backup")?;
            }

            // Read file bytes
            let snapshot_bytes =
                std::fs::read(&temp_path).context("Failed to read snapshot file")?;

            // Clean up temp file
            let _ = std::fs::remove_file(&temp_path);

            snapshot_bytes
        };

        info!(
            "Snapshot created: {} bytes at sequence {}",
            snapshot_data.len(),
            state.sequence
        );

        Ok(Snapshot {
            data: snapshot_data,
            timestamp: SystemTime::now(),
            sequence: state.sequence,
        })
    }
}

impl Drop for SessionMonitor {
    fn drop(&mut self) {
        debug!("Dropping SessionMonitor");

        // Signal flush thread to stop
        let _ = self.flush_shutdown_tx.send(());

        // Wait for flush thread to finish
        if let Some(handle) = self.flush_handle.take() {
            if let Err(e) = handle.join() {
                error!("Flush thread panicked: {:?}", e);
            }
        }
    }
}
