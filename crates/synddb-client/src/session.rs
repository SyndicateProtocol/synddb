//! SQLite Session Extension integration

use anyhow::{Context, Result};
use crossbeam_channel::Sender;
use rusqlite::hooks::Action;
use rusqlite::session::Session;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;
use tracing::{debug, trace};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Changeset {
    /// Raw changeset bytes from SQLite
    pub data: Vec<u8>,
    /// Sequence number (monotonic)
    pub sequence: u64,
    /// Timestamp when captured
    pub timestamp: SystemTime,
}

/// Shared state between session monitor and hook
struct SessionState {
    session: Session<'static>,
    changeset_tx: Sender<Changeset>,
    sequence: u64,
}

pub struct SessionMonitor {
    state: Arc<Mutex<SessionState>>,
}

impl SessionMonitor {
    pub fn new(conn: &'static Connection, changeset_tx: Sender<Changeset>) -> Result<Self> {
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

        debug!("SessionMonitor created");

        Ok(Self { state })
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
                // We'll rely on periodic flushing or explicit flush calls.
            },
        ));

        debug!("SessionMonitor started");
        Ok(())
    }

    /// Capture and send all changes since last flush
    pub fn flush(&self) -> Result<()> {
        trace!("Flushing session changes");

        let mut state = self.state.lock().unwrap();

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
}
