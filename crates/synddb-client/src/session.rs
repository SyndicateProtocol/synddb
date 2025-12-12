//! `SQLite` Session Extension integration
//!
//! # Thread Safety
//!
//! The Session Extension contains raw pointers that are NOT thread-safe. To avoid races:
//! - `SessionState` is stored in thread-local storage, accessed only from the main thread
//! - Background threads only receive `Vec<u8>` bytes through channels
//! - `publish()` and `snapshot()` have debug assertions verifying the calling thread
//!
//! # Publishing
//!
//! Changesets must be published explicitly via `publish()`. Automatic publishing via
//! UPDATE or COMMIT hooks is not possible because `SQLite`'s session extension requires reading from
//! the database during extraction, which is not allowed inside hook callbacks.

use anyhow::{Context, Result};
use crossbeam_channel::Sender;
use rusqlite::{backup::Backup, hooks::Action, session::Session, Connection};
use serde::{Deserialize, Serialize};
use std::{
    cell::RefCell,
    thread::{self, ThreadId},
    time::{Duration, SystemTime},
};
use tracing::{debug, error, info, trace};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Changeset {
    /// Raw changeset bytes from `SQLite`
    pub data: Vec<u8>,
    /// Sequence number (monotonic)
    pub sequence: u64,
    /// Timestamp when captured
    pub timestamp: SystemTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    /// Complete database snapshot as `SQLite` database file bytes
    pub data: Vec<u8>,
    /// Timestamp when snapshot was captured
    pub timestamp: SystemTime,
    /// Sequence number at the time of snapshot
    /// (changesets with sequence >= this should be applied after snapshot)
    pub sequence: u64,
}

/// Session state that lives on the main thread only.
///
/// This is stored in a thread-local `RefCell` and accessed only from the main thread.
/// The Session is never shared across threads - only the extracted `Vec<u8>` bytes
/// are sent to background threads via channels.
///
/// Note: We intentionally do NOT implement `Send` for `SessionState`.
/// The Session contains raw pointers that are not safe to share across threads.
/// All Session access happens on the main thread.
struct SessionState {
    session: Session<'static>,
    changeset_tx: Sender<Changeset>,
    sequence: u64,
    conn: &'static Connection,
    snapshot_interval: u64,
    changesets_since_snapshot: u64,
    snapshot_tx: Option<Sender<Snapshot>>,
    schema_changed: bool,
    /// Flag to indicate changes have occurred since last publish
    has_changes: bool,
}

thread_local! {
    /// Thread-local storage for session state.
    /// Note: This is thread-local, so each thread gets its own instance.
    /// The SessionMonitor API is designed so that only the main thread accesses SESSION_STATE.
    static SESSION_STATE: RefCell<Option<SessionState>> = const { RefCell::new(None) };
}

#[derive(Debug)]
pub struct SessionMonitor {
    /// Connection reference for snapshot creation
    conn: &'static Connection,
    /// Thread that created this monitor (for debug assertions)
    owner_thread: ThreadId,
}

impl SessionMonitor {
    pub(crate) fn new(
        conn: &'static Connection,
        changeset_tx: Sender<Changeset>,
        snapshot_interval: u64,
        snapshot_tx: Option<Sender<Snapshot>>,
    ) -> Result<Self> {
        debug!("Initializing SQLite Session Extension");

        if snapshot_interval > 0 {
            info!(
                "Automatic snapshots enabled: every {} changesets",
                snapshot_interval
            );
        } else {
            info!("Automatic snapshots disabled");
        }

        // Create a session attached to the main database
        let mut session = Session::new(conn).context("Failed to create SQLite session")?;

        // Attach to all tables (None means all tables)
        session
            .attach(None::<&str>)
            .context("Failed to attach session to tables")?;

        debug!("Session attached to all tables");

        // Store session state in thread-local storage
        SESSION_STATE.with(|state| {
            *state.borrow_mut() = Some(SessionState {
                session,
                changeset_tx,
                sequence: 0,
                conn,
                snapshot_interval,
                changesets_since_snapshot: 0,
                snapshot_tx,
                schema_changed: false,
                has_changes: false,
            });
        });

        debug!("SessionMonitor created");

        Ok(Self {
            conn,
            owner_thread: thread::current().id(),
        })
    }

    /// Install update hooks for change detection.
    ///
    /// The hook fires for every row modified (INSERT/UPDATE/DELETE). To minimize overhead
    /// in batch operations, flags are checked before writing (read is cheaper than write).
    /// E.g. for a 10,000 row batch: 1 write + 9,999 reads instead of 10,000 writes.
    ///
    /// **Note**: Changeset extraction cannot happen inside hooks (`update_hook` or `commit_hook`)
    /// because `SQLite`'s session extension requires reading from the database, which is not
    /// allowed during hook callbacks. Instead, call `publish()` after transactions complete.
    pub(crate) fn start(&self, conn: &Connection) -> Result<()> {
        debug!("Installing hooks for change detection");

        // Install update hook to detect when changes occur
        conn.update_hook(Some(
            |action: Action, _db: &str, table: &str, rowid: i64| {
                trace!(
                    "Update hook: {:?} on table {} rowid {}",
                    action,
                    table,
                    rowid
                );

                SESSION_STATE.with(|state| {
                    if let Ok(mut guard) = state.try_borrow_mut() {
                        if let Some(ref mut s) = *guard {
                            // Skip if already marked (avoid redundant writes in batch operations)
                            if !s.has_changes {
                                s.has_changes = true;
                            }

                            // Detect schema changes by monitoring sqlite_schema table
                            if !s.schema_changed
                                && (table == "sqlite_schema" || table == "sqlite_master")
                            {
                                info!(
                                    "Schema change detected ({:?} on {}), will trigger snapshot",
                                    action, table
                                );
                                s.schema_changed = true;
                            }
                        }
                    }
                });
            },
        ));

        debug!("SessionMonitor started with change detection");
        Ok(())
    }

    /// Extract changeset from session and send to background thread.
    /// This must be called from the main thread.
    fn extract_and_send_changeset(state: &mut SessionState) -> Result<()> {
        // If schema changed, create and send snapshot BEFORE capturing changesets
        if state.schema_changed && state.conn.is_autocommit() {
            info!("Creating immediate snapshot for schema change");

            match Self::create_snapshot_internal(state) {
                Ok(snapshot) => {
                    if let Some(ref snapshot_tx) = state.snapshot_tx {
                        if let Err(e) = snapshot_tx.try_send(snapshot) {
                            error!("Failed to send schema change snapshot: {}", e);
                        } else {
                            info!("Schema change snapshot sent at sequence {}", state.sequence);
                            state.changesets_since_snapshot = 0;
                        }
                    }
                    state.schema_changed = false;
                }
                Err(e) => {
                    error!("Failed to create schema change snapshot: {}", e);
                }
            }
        }

        // Only extract if there might be changes and we're not in a transaction
        if !state.has_changes {
            return Ok(());
        }

        // Check if we're in autocommit mode (no active transaction)
        // If a transaction is active, we can't safely extract changesets
        if !state.conn.is_autocommit() {
            trace!("Skipping changeset extraction - transaction active");
            return Ok(());
        }

        // Extract changeset bytes from session
        let mut changeset_data = Vec::new();
        state
            .session
            .changeset_strm(&mut changeset_data)
            .context("Failed to get changeset from session")?;

        // Reset change flag now that we've extracted
        state.has_changes = false;

        if changeset_data.is_empty() {
            trace!("No changes to publish");
            return Ok(());
        }

        trace!("Captured changeset: {} bytes", changeset_data.len());

        let changeset = Changeset {
            data: changeset_data,
            sequence: state.sequence,
            timestamp: SystemTime::now(),
        };

        state.sequence += 1;
        state.changesets_since_snapshot += 1;

        // Send changeset to background thread (non-blocking)
        if let Err(e) = state.changeset_tx.try_send(changeset) {
            error!("Failed to send changeset: {}", e);
        }

        // Check if we should create a regular interval-based snapshot
        if state.snapshot_interval > 0
            && state.changesets_since_snapshot >= state.snapshot_interval
            && state.conn.is_autocommit()
        {
            info!(
                "Snapshot threshold reached ({} changesets), creating automatic snapshot",
                state.changesets_since_snapshot
            );

            match Self::create_snapshot_internal(state) {
                Ok(snapshot) => {
                    if let Some(ref snapshot_tx) = state.snapshot_tx {
                        if let Err(e) = snapshot_tx.try_send(snapshot) {
                            error!("Failed to send interval snapshot: {}", e);
                        } else {
                            info!("Automatic snapshot sent at sequence {}", state.sequence);
                            state.changesets_since_snapshot = 0;
                        }
                    }
                }
                Err(e) => {
                    error!("Failed to create interval snapshot: {}", e);
                }
            }
        }

        Ok(())
    }

    /// Internal method to create snapshot
    fn create_snapshot_internal(state: &SessionState) -> Result<Snapshot> {
        // Check if the connection is in auto-commit mode (no active transaction)
        if !state.conn.is_autocommit() {
            return Err(anyhow::anyhow!(
                "Cannot create snapshot while a transaction is active"
            ));
        }

        // Serialize database to bytes using a temporary file
        let temp_path =
            std::env::temp_dir().join(format!("synddb_snapshot_{}.db", uuid::Uuid::new_v4()));

        // Backup source to temporary file
        {
            let mut file_conn =
                Connection::open(&temp_path).context("Failed to create temporary snapshot file")?;

            let backup =
                Backup::new(state.conn, &mut file_conn).context("Failed to initialize backup")?;

            backup
                .run_to_completion(5, Duration::from_millis(250), None)
                .context("Failed to complete backup")?;
        }

        // Read file bytes
        let snapshot_bytes = std::fs::read(&temp_path).context("Failed to read snapshot file")?;

        // Clean up temp file
        let _ = std::fs::remove_file(&temp_path);

        Ok(Snapshot {
            data: snapshot_bytes,
            timestamp: SystemTime::now(),
            sequence: state.sequence,
        })
    }

    /// Trigger changeset extraction and publishing.
    /// Call this after committing transactions to send changesets to the sequencer.
    /// Must be called from the same thread that created the `SessionMonitor`.
    pub(crate) fn publish(&self) -> Result<()> {
        debug_assert_eq!(
            thread::current().id(),
            self.owner_thread,
            "publish() must be called from the same thread that created SyndDB"
        );

        SESSION_STATE.with(|state| {
            let mut guard = state.borrow_mut();
            guard.as_mut().map_or_else(
                || {
                    Err(anyhow::anyhow!(
                        "Session state not initialized - internal error"
                    ))
                },
                Self::extract_and_send_changeset,
            )
        })
    }

    /// Create a complete snapshot of the database.
    /// Must be called from the same thread that created the `SessionMonitor`.
    pub(crate) fn snapshot(&self) -> Result<Snapshot> {
        debug_assert_eq!(
            thread::current().id(),
            self.owner_thread,
            "snapshot() must be called from the same thread that created SyndDB"
        );

        info!("Creating manual database snapshot");

        SESSION_STATE.with(|state| {
            let guard = state.borrow();
            if let Some(ref s) = *guard {
                let snapshot = Self::create_snapshot_internal(s)?;
                info!(
                    "Manual snapshot created: {} bytes at sequence {}",
                    snapshot.data.len(),
                    snapshot.sequence
                );
                Ok(snapshot)
            } else {
                Err(anyhow::anyhow!(
                    "Session state not initialized - internal error"
                ))
            }
        })
    }
}

impl Drop for SessionMonitor {
    fn drop(&mut self) {
        debug!("Dropping SessionMonitor");

        // Clear the update hook
        self.conn.update_hook(None::<fn(Action, &str, &str, i64)>);

        // Final extraction of any pending changesets
        SESSION_STATE.with(|state| {
            if let Ok(mut guard) = state.try_borrow_mut() {
                if let Some(ref mut s) = *guard {
                    if let Err(e) = Self::extract_and_send_changeset(s) {
                        debug!("Final changeset extraction: {}", e);
                    }
                }
            } else {
                error!("Failed to borrow session state during drop - concurrent access detected");
            }
        });

        // Clear the thread-local state
        SESSION_STATE.with(|state| {
            if let Ok(mut guard) = state.try_borrow_mut() {
                *guard = None;
            }
        });

        debug!("SessionMonitor dropped");
    }
}
