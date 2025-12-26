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
    /// Flag to indicate changes have occurred since last publish
    has_changes: bool,
    /// Hash of `sqlite_master` to detect schema changes between publishes.
    /// If the schema changes (DDL executed), we automatically trigger a snapshot.
    last_schema_hash: u64,
}

impl SessionState {
    /// Recreate the session to clear accumulated changes.
    ///
    /// The `SQLite` Session Extension does NOT reset after `changeset_strm()` extraction.
    /// Each subsequent call returns ALL changes since session creation. To get only
    /// new changes in the next extraction, we must drop the old session and create
    /// a fresh one. See: <https://sqlite.org/session/sqlite3session_changeset.html>
    fn recreate_session(&mut self) -> Result<()> {
        // Drop the old session by replacing it
        let mut new_session =
            Session::new(self.conn).context("Failed to create new SQLite session")?;

        // Attach to all tables (None means all tables)
        new_session
            .attach(None::<&str>)
            .context("Failed to attach new session to tables")?;

        self.session = new_session;
        debug!("Session recreated to clear accumulated changes");
        Ok(())
    }

    /// Compute a hash of the current schema (`sqlite_master` contents).
    ///
    /// This is used to detect schema changes (DDL) that happen outside of `execute_ddl()`.
    /// When the schema hash changes, we automatically trigger a snapshot to ensure
    /// validators have the updated schema.
    fn compute_schema_hash(conn: &Connection) -> u64 {
        use std::{
            collections::hash_map::DefaultHasher,
            hash::{Hash, Hasher},
        };

        let mut hasher = DefaultHasher::new();

        // Query all schema objects (tables, indexes, triggers, views)
        // Order by type and name for deterministic hashing
        let result: Result<Vec<(String, String, String)>, _> = conn
            .prepare(
                "SELECT type, name, sql FROM sqlite_master \
                 WHERE name NOT LIKE 'sqlite_%' \
                 ORDER BY type, name",
            )
            .and_then(|mut stmt| {
                stmt.query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                    ))
                })
                .and_then(|rows| rows.collect())
            });

        match result {
            Ok(schema_items) => {
                for (type_, name, sql) in schema_items {
                    type_.hash(&mut hasher);
                    name.hash(&mut hasher);
                    sql.hash(&mut hasher);
                }
            }
            Err(e) => {
                // If we can't read the schema, use a sentinel value
                // This shouldn't happen in practice
                debug!("Failed to read schema for hashing: {}", e);
                0u64.hash(&mut hasher);
            }
        }

        hasher.finish()
    }
}

thread_local! {
    /// Thread-local storage for session state.
    /// Note: This is thread-local, so each thread gets its own instance.
    /// The SessionMonitor API is designed so that only the main thread accesses SESSION_STATE.
    static SESSION_STATE: RefCell<Option<SessionState>> = const { RefCell::new(None) };

    /// Flag indicating we're inside an `execute_ddl()` call.
    /// When true, schema changes are expected and handled properly.
    /// When false and a schema change is detected, we warn or panic.
    static IN_EXECUTE_DDL: RefCell<bool> = const { RefCell::new(false) };
}

/// Set the `IN_EXECUTE_DDL` flag for the duration of a closure.
/// This is called by `SyndDB::execute_ddl()` to indicate that schema changes
/// are expected and should not trigger warnings.
pub(crate) fn with_ddl_context<F, R>(f: F) -> R
where
    F: FnOnce() -> R,
{
    IN_EXECUTE_DDL.with(|flag| *flag.borrow_mut() = true);
    let result = f();
    IN_EXECUTE_DDL.with(|flag| *flag.borrow_mut() = false);
    result
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

        // Compute initial schema hash for change detection
        let initial_schema_hash = SessionState::compute_schema_hash(conn);
        debug!("Initial schema hash: {:016x}", initial_schema_hash);

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
                has_changes: false,
                last_schema_hash: initial_schema_hash,
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

                            // Note: We previously tried to detect schema changes by monitoring
                            // sqlite_schema/sqlite_master tables here. However, SQLite's update
                            // hook does NOT fire for DDL operations (CREATE/ALTER/DROP) - it only
                            // fires for INSERT/UPDATE/DELETE on user tables.
                            //
                            // DDL crash recovery is now handled in execute_ddl() which writes
                            // a marker before execution and clears it after the snapshot is created.
                            // Direct DDL via connection().execute() cannot be detected and is
                            // not covered by the crash recovery mechanism.
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

        // SCHEMA CHANGE DETECTION: Check if schema changed since last publish.
        // SQLite's update hook doesn't fire for DDL, so we detect schema changes
        // by comparing the hash of sqlite_master. If the schema changed (DDL was
        // executed), we MUST publish a snapshot first so validators have the
        // updated schema before receiving changesets that reference it.
        let current_schema_hash = SessionState::compute_schema_hash(state.conn);
        if current_schema_hash != state.last_schema_hash {
            info!(
                "Schema change detected (hash {:016x} -> {:016x}), publishing snapshot",
                state.last_schema_hash, current_schema_hash
            );

            // Create and send snapshot before processing changesets
            match Self::create_snapshot_internal(state) {
                Ok(snapshot) => {
                    if let Some(ref snapshot_tx) = state.snapshot_tx {
                        if let Err(e) = snapshot_tx.try_send(snapshot.clone()) {
                            error!("Failed to send schema-change snapshot: {}", e);
                        } else {
                            info!(
                                "Schema-change snapshot sent: {} bytes at sequence {}",
                                snapshot.data.len(),
                                snapshot.sequence
                            );
                            // Increment sequence so subsequent changesets have a later sequence
                            // This ensures validators apply snapshot before changesets
                            state.sequence += 1;
                            state.changesets_since_snapshot = 0;
                        }
                    } else {
                        // No snapshot channel configured - log warning
                        // This happens when snapshot_interval is 0 (disabled)
                        // The snapshot is still important for schema changes
                        info!(
                            "Schema changed but no snapshot channel configured. \
                             Consider enabling snapshot_interval or calling publish_snapshot() manually."
                        );
                    }
                }
                Err(e) => {
                    error!("Failed to create schema-change snapshot: {}", e);
                }
            }

            // Update the stored schema hash
            state.last_schema_hash = current_schema_hash;
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

        info!(
            sequence = state.sequence,
            bytes = changeset_data.len(),
            "Captured changeset"
        );

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

        // Recreate session to clear accumulated changes.
        // The SQLite Session Extension does NOT reset after changeset_strm() -
        // it accumulates all changes since session creation. We must recreate
        // the session so the next extraction only captures NEW changes.
        state.recreate_session()?;

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
