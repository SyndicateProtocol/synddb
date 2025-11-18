//! SQLite Session Extension integration

use anyhow::{Context, Result};
use crossbeam_channel::Sender;
use rusqlite::hooks::Action;
use rusqlite::session::Session;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime};
use tracing::{debug, error, trace};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Changeset {
    /// Raw changeset bytes from SQLite
    pub data: Vec<u8>,
    /// Sequence number (monotonic)
    pub sequence: u64,
    /// Timestamp when captured
    pub timestamp: SystemTime,
}

/// Shared state between session monitor and flush thread
struct SessionState {
    session: Session<'static>,
    changeset_tx: Sender<Changeset>,
    sequence: u64,
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
