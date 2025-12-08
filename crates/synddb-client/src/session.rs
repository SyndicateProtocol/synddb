//! `SQLite` Session Extension integration
//!
//! This module provides thread-safe changeset capture using `SQLite`'s Session Extension.
//!
//! # Thread Safety Architecture
//!
//! The Session Extension contains raw pointers that are NOT safe to access from multiple
//! threads, even with a Mutex. The Mutex only protects Rust-level data races, but `SQLite`'s
//! internal state can be corrupted when:
//! - Main thread is executing SQL via `conn.execute()`
//! - Background thread calls `session.changeset_strm()`
//!
//! To avoid this race condition, we ensure all Session operations happen on the main thread:
//! - Changeset extraction happens only when `publish()` is called from the main thread
//! - The `publish()` call is triggered by the application after completing transactions
//! - Only `Vec<u8>` bytes (which are safe to Send) are sent to background threads
//!
//! This eliminates the need for `unsafe impl Send` on any Session-containing type.

use anyhow::{Context, Result};
use crossbeam_channel::Sender;
use rusqlite::backup::Backup;
use rusqlite::hooks::Action;
use rusqlite::session::Session;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, ThreadId};
use std::time::{Duration, SystemTime};
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

// Note: We intentionally do NOT implement Send for SessionState.
// The Session contains raw pointers that are not safe to share across threads.
// All Session access happens on the main thread.

thread_local! {
    /// Thread-local storage for session state.
    /// Note: This is thread-local, so each thread gets its own instance.
    /// The SessionMonitor API is designed so that only the main thread accesses SESSION_STATE.
    static SESSION_STATE: RefCell<Option<SessionState>> = const { RefCell::new(None) };
}

/// Signal type for publish requests from timer thread
#[derive(Debug)]
struct PublishSignal {
    should_publish: AtomicBool,
}

#[derive(Debug)]
pub struct SessionMonitor {
    /// Connection reference for snapshot creation
    conn: &'static Connection,
    /// Shutdown flag for timer thread
    shutdown: Arc<AtomicBool>,
    /// Timer thread handle
    timer_handle: Option<thread::JoinHandle<()>>,
    /// Publish signal shared with timer
    publish_signal: Arc<PublishSignal>,
    /// Thread that created this monitor (for debug assertions)
    owner_thread: ThreadId,
}

impl SessionMonitor {
    pub(crate) fn new(
        conn: &'static Connection,
        changeset_tx: Sender<Changeset>,
        publish_interval: Duration,
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

        let shutdown = Arc::new(AtomicBool::new(false));
        let publish_signal = Arc::new(PublishSignal {
            should_publish: AtomicBool::new(false),
        });

        // Start a timer thread that just sets a flag periodically
        // The actual changeset extraction happens when the main thread checks this flag
        let shutdown_clone = Arc::clone(&shutdown);
        let signal_clone = Arc::clone(&publish_signal);
        let timer_handle = thread::spawn(move || {
            debug!("Timer thread started with interval {:?}", publish_interval);
            while !shutdown_clone.load(Ordering::Relaxed) {
                thread::sleep(publish_interval);
                signal_clone.should_publish.store(true, Ordering::Release);
            }
            debug!("Timer thread stopped");
        });

        debug!("SessionMonitor created");

        Ok(Self {
            conn,
            shutdown,
            timer_handle: Some(timer_handle),
            publish_signal,
            owner_thread: thread::current().id(),
        })
    }

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

                // Detect schema changes by monitoring sqlite_schema table
                if table == "sqlite_schema" || table == "sqlite_master" {
                    info!(
                        "Schema change detected ({:?} on {}), will trigger snapshot",
                        action, table
                    );

                    SESSION_STATE.with(|state| {
                        if let Ok(mut guard) = state.try_borrow_mut() {
                            if let Some(ref mut s) = *guard {
                                s.schema_changed = true;
                            }
                        }
                    });
                }

                // Mark that changes have occurred
                SESSION_STATE.with(|state| {
                    if let Ok(mut guard) = state.try_borrow_mut() {
                        if let Some(ref mut s) = *guard {
                            s.has_changes = true;
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

    /// Manually trigger changeset extraction and publishing.
    /// This can be called to publish immediately after critical transactions.
    /// Must be called from the same thread that created the SessionMonitor.
    pub(crate) fn publish(&self) -> Result<()> {
        debug_assert_eq!(
            thread::current().id(),
            self.owner_thread,
            "publish() must be called from the same thread that created SyndDB"
        );

        // Also check the timer signal in case it was set
        self.publish_signal
            .should_publish
            .store(false, Ordering::Release);

        SESSION_STATE.with(|state| {
            let mut guard = state.borrow_mut();
            guard.as_mut().map_or_else(|| Err(anyhow::anyhow!("Session state not initialized - internal error" )), Self::extract_and_send_changeset)
        })
    }

    /// Create a complete snapshot of the database.
    /// Must be called from the same thread that created the SessionMonitor.
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
                Err(anyhow::anyhow!("Session state not initialized - internal error"))
            }
        })
    }
}

impl Drop for SessionMonitor {
    fn drop(&mut self) {
        debug!("Dropping SessionMonitor");

        // Signal timer thread to stop
        self.shutdown.store(true, Ordering::Release);

        // Wait for timer thread
        if let Some(handle) = self.timer_handle.take() {
            let _ = handle.join();
        }

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
