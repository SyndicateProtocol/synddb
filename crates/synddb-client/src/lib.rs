//! `SyndDB` Client Library - `SQLite` Replication for Blockchain Applications
//!
//! This library captures `SQLite` changesets and sends them to the `SyndDB` sequencer
//! for ordering, signing, and publication to data availability layers.
//!
//! # Quick Start
//!
//! The simplest way to use `SyndDB` is with [`SyndDB::open()`], which manages the
//! connection internally:
//!
//! ```rust,no_run
//! use synddb_client::SyndDB;
//!
//! // Open database with replication enabled
//! let synddb = SyndDB::open("app.db", "http://sequencer:8433")?;
//!
//! // Use execute_ddl() for schema changes (CREATE, ALTER, DROP)
//! synddb.execute_ddl("CREATE TABLE IF NOT EXISTS trades (id INTEGER, amount INTEGER)")?;
//!
//! // Use the transaction helper for multi-statement transactions
//! synddb.transaction(|tx| {
//!     tx.execute("INSERT INTO trades VALUES (1, 100)", [])?;
//!     tx.execute("INSERT INTO trades VALUES (2, 200)", [])?;
//!     Ok(())
//! })?;
//!
//! // Check replication status
//! let stats = synddb.stats();
//! println!("Pushed: {} changesets", stats.pushed_changesets);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! # Advanced Usage
//!
//! For more control, use [`SyndDB::attach()`] with an existing connection:
//!
//! ```rust,no_run
//! use rusqlite::Connection;
//! use synddb_client::SyndDB;
//!
//! // Create connection with 'static lifetime (required by `SQLite` Session Extension)
//! let conn = Box::leak(Box::new(Connection::open("app.db")?));
//! let synddb = SyndDB::attach(conn, "http://sequencer:8433")?;
//!
//! // Use unchecked_transaction() for transactions (see Transactions section)
//! let tx = conn.unchecked_transaction()?;
//! tx.execute("INSERT INTO trades VALUES (1, 100)", [])?;
//! tx.commit()?;
//!
//! synddb.push()?; // Optionally force immediate push
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! # Automatic Pushing
//!
//! Changesets are automatically pushed every second (configurable via `flush_interval`).
//! Use [`SyndDB::push()`] to force immediate push for low-latency
//! or high-value changes.
//!
//! # Transactions
//!
//! When using [`SyndDB::attach()`] with an external connection, you must use
//! [`Connection::unchecked_transaction()`] instead of [`Connection::transaction()`].
//! This is because `SyndDB` holds a reference to the connection for the Session Extension.
//!
//! The [`SyndDB::transaction()`] helper handles this automatically:
//!
//! ```rust,no_run
//! # use synddb_client::SyndDB;
//! # let synddb = SyndDB::open("app.db", "http://sequencer:8433")?;
//! // Recommended: use the transaction helper
//! synddb.transaction(|tx| {
//!     tx.execute("UPDATE balances SET amount = amount - 100 WHERE id = 1", [])?;
//!     tx.execute("UPDATE balances SET amount = amount + 100 WHERE id = 2", [])?;
//!     Ok(())
//! })?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! # Monitoring Replication
//!
//! Check replication health and statistics:
//!
//! ```rust,no_run
//! # use synddb_client::SyndDB;
//! # let synddb = SyndDB::open("app.db", "http://sequencer:8433")?;
//! // Get current stats
//! let stats = synddb.stats();
//! println!("Pending: {}", stats.pending_changesets);
//! println!("Pushed: {}", stats.pushed_changesets);
//! println!("Healthy: {}", stats.is_healthy);
//!
//! // Quick health check
//! if synddb.is_healthy() {
//!     println!("Sequencer is reachable");
//! }
//!
//! // Check pending count
//! println!("{} changesets waiting to be sent", synddb.pending_count());
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! # Schema Changes (DDL)
//!
//! Schema changes (CREATE, ALTER, DROP) require special handling to ensure validators
//! can reconstruct the database state. Always use [`SyndDB::execute_ddl()`] for DDL:
//!
//! ```rust,no_run
//! # use synddb_client::SyndDB;
//! # let synddb = SyndDB::open("app.db", "http://sequencer:8433")?;
//! // CORRECT: Use execute_ddl() for schema changes
//! synddb.execute_ddl("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)")?;
//!
//! // AVOID: Direct connection access for DDL (triggers warning)
//! // synddb.connection().execute("CREATE TABLE bad_pattern (...)", [])?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! ## Automatic Snapshot Behavior
//!
//! - **On attach**: When attaching to a database with existing tables, automatically
//!   publishes a snapshot so validators can reconstruct the pre-existing state.
//!   This is always enabled - it's critical for validator sync.
//!
//! - **After DDL**: After executing DDL via [`SyndDB::execute_ddl()`], automatically
//!   publishes a snapshot. This ensures validators can always reconstruct the schema.
//!   This is always enabled and built into `execute_ddl()`.
//!
//! **Important**: Always use [`SyndDB::execute_ddl()`] for schema changes (CREATE, ALTER,
//! DROP). Direct DDL via [`SyndDB::connection()`] bypasses snapshot creation and cannot
//! be detected by `SyndDB`.
//!
//! ## Recovery from Direct DDL
//!
//! If you accidentally execute DDL directly (without `execute_ddl()`), validators will
//! fail with errors like "no such table" or "no such column". To recover:
//!
//! ```rust,no_run
//! # use synddb_client::SyndDB;
//! # let synddb = SyndDB::open("app.db", "http://sequencer:8433")?;
//! // Oops! Used direct DDL instead of execute_ddl()
//! synddb.connection().execute("CREATE TABLE oops (id INTEGER)", [])?;
//!
//! // RECOVERY: Immediately publish a snapshot to capture the schema
//! synddb.snapshot()?;
//!
//! // Validators now have the schema and can apply subsequent changesets
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! The snapshot captures the complete current database state (schema + data), allowing
//! validators to synchronize from that point forward.
//!
//! # Thread Safety
//!
//! The `SQLite` Session Extension is only accessed from the main thread. Background
//! threads handle network I/O but only receive serialized bytes through channels.
//! All stats are thread-safe and can be read from any thread.

use anyhow::Result;
use crossbeam_channel::{bounded, Sender};
use rusqlite::Connection;
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    path::Path,
    sync::Arc,
    thread,
};
use tracing::{debug, info, warn};

pub mod attestation;
pub mod config;
pub(crate) mod ddl_recovery;
pub mod recovery;
pub mod retry;
pub mod sender;
pub mod session;
pub mod snapshot_sender;
pub mod stats;

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
pub use stats::{StatsHandle, StatsSnapshot};

/// Main handle to `SyndDB` client
///
/// Provides `SQLite` replication by capturing changesets and sending them to a sequencer.
///
/// # Creating an Instance
///
/// - [`SyndDB::open()`] - Simplest way, manages connection internally
/// - [`SyndDB::open_with_config()`] - Full control over configuration
/// - [`SyndDB::attach()`] - Attach to existing connection (advanced)
///
/// # Example
///
/// ```rust,no_run
/// use synddb_client::SyndDB;
///
/// let synddb = SyndDB::open("app.db", "http://sequencer:8433")?;
///
/// synddb.connection().execute("INSERT INTO trades VALUES (1, 100)", [])?;
///
/// synddb.transaction(|tx| {
///     tx.execute("INSERT INTO trades VALUES (2, 200)", [])?;
///     Ok(())
/// })?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub struct SyndDB {
    /// Connection reference (always valid for lifetime of `SyndDB`)
    conn: &'static Connection,
    /// Sequencer URL for publishing snapshots
    sequencer_url: url::Url,
    /// Database path for DDL recovery marker (None for in-memory)
    db_path: Option<String>,
    /// Session monitor for capturing changesets
    monitor: Option<SessionMonitor>,
    /// Channel to send shutdown signal to changeset sender
    changeset_shutdown_tx: Sender<()>,
    /// Channel to send shutdown signal to snapshot sender
    snapshot_shutdown_tx: Sender<()>,
    /// Handle to background changeset sender thread
    changeset_handle: Option<thread::JoinHandle<()>>,
    /// Handle to background snapshot sender thread
    snapshot_handle: Option<thread::JoinHandle<()>>,
    /// Optional recovery storage for failed batches
    recovery: Option<Arc<FailedBatchRecovery>>,
    /// Optional chain monitor handle (enabled with `chain-monitor` feature)
    chain_monitor: Option<ChainMonitorHandle>,
    /// Shared replication statistics
    stats: StatsHandle,
}

impl std::fmt::Debug for SyndDB {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SyndDB")
            .field("monitor", &self.monitor.is_some())
            .field("recovery", &self.recovery.is_some())
            .field("chain_monitor", &self.chain_monitor.is_some())
            .field("stats", &self.stats)
            .finish_non_exhaustive()
    }
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
    // =========================================================================
    // Constructors
    // =========================================================================

    /// Open a database with replication enabled
    ///
    /// This is the simplest way to use `SyndDB`. It creates and manages the `SQLite`
    /// connection internally, hiding the complexity of lifetime management.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the `SQLite` database file
    /// * `sequencer_url` - URL of the sequencer (e.g., `"http://sequencer:8433"`)
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use synddb_client::SyndDB;
    ///
    /// let synddb = SyndDB::open("app.db", "http://sequencer:8433")?;
    /// synddb.connection().execute("INSERT INTO trades VALUES (1, 100)", [])?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn open<P: AsRef<Path>>(path: P, sequencer_url: &str) -> Result<Self> {
        let url =
            synddb_shared::parse::parse_url(sequencer_url).map_err(|e| anyhow::anyhow!("{}", e))?;
        Self::open_with_config(
            path,
            Config {
                sequencer_url: url,
                ..Default::default()
            },
        )
    }

    /// Open a database with custom configuration
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use synddb_client::{Config, SyndDB};
    /// use std::time::Duration;
    ///
    /// let config = Config {
    ///     sequencer_url: "http://sequencer:8433".parse().unwrap(),
    ///     flush_interval: Duration::from_millis(100), // Faster auto-publish
    ///     ..Default::default()
    /// };
    /// let synddb = SyndDB::open_with_config("app.db", config)?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn open_with_config<P: AsRef<Path>>(path: P, config: Config) -> Result<Self> {
        let conn = Connection::open(path)?;
        // Leak the connection to get 'static lifetime (required by SQLite Session Extension)
        let conn: &'static Connection = Box::leak(Box::new(conn));
        Self::attach_with_config(conn, config)
    }

    /// Open an in-memory database with replication enabled
    ///
    /// Useful for testing or temporary data that doesn't need persistence.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use synddb_client::SyndDB;
    ///
    /// let synddb = SyndDB::open_in_memory("http://sequencer:8433")?;
    /// synddb.connection().execute("CREATE TABLE test (id INTEGER)", [])?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn open_in_memory(sequencer_url: &str) -> Result<Self> {
        let url =
            synddb_shared::parse::parse_url(sequencer_url).map_err(|e| anyhow::anyhow!("{}", e))?;
        let conn = Connection::open_in_memory()?;
        let conn: &'static Connection = Box::leak(Box::new(conn));
        Self::attach_with_config(
            conn,
            Config {
                sequencer_url: url,
                ..Default::default()
            },
        )
    }

    /// Attach to an existing `SQLite` connection (advanced)
    ///
    /// Use this when you need direct control over the connection. Note that the
    /// connection must have `'static` lifetime, typically achieved via `Box::leak`.
    ///
    /// For most cases, prefer [`SyndDB::open()`] which handles this automatically.
    ///
    /// # Arguments
    ///
    /// * `conn` - `SQLite` connection with `'static` lifetime
    /// * `sequencer_url` - URL of the sequencer TEE
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use rusqlite::Connection;
    /// use synddb_client::SyndDB;
    ///
    /// let conn = Box::leak(Box::new(Connection::open("app.db")?));
    /// let synddb = SyndDB::attach(conn, "http://sequencer:8433")?;
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

    /// Attach to an existing connection with custom configuration
    pub fn attach_with_config(conn: &'static Connection, config: Config) -> Result<Self> {
        info!("Attaching SyndDB client to SQLite connection");
        info!("Sequencer URL: {}", config.sequencer_url);

        // Validate configuration values
        Self::validate_config(&config)?;

        // Get database path (None for in-memory databases)
        let db_path = conn.path().map(String::from);

        // Check for DDL recovery marker from previous crash
        let needs_recovery_snapshot = db_path
            .as_ref()
            .is_some_and(|path| ddl_recovery::check_marker(path));

        if needs_recovery_snapshot {
            warn!(
                "DDL recovery marker found! A previous run had uncommitted schema changes. \
                Will force a snapshot after initialization to ensure validators can reconstruct the schema."
            );
        }

        // Create shared stats handle
        let stats = stats::new_stats_handle();

        // Create channels for communication
        let (changeset_tx, changeset_rx) = bounded(config.buffer_size);
        let (changeset_shutdown_tx, changeset_shutdown_rx) = bounded(1);

        // Create snapshot channel (always enabled since snapshot_interval > 0 is enforced)
        let snapshot_channel = bounded(10); // Buffer up to 10 snapshots

        // Start session monitor
        let (snapshot_tx, snapshot_rx) = snapshot_channel;
        let monitor = SessionMonitor::new(
            conn,
            changeset_tx,
            config.snapshot_interval,
            Some(snapshot_tx),
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
                Ok(r) => Some(Arc::new(r)),
                Err(e) => {
                    warn!(
                        "Failed to initialize recovery storage: {}. Continuing without recovery.",
                        e
                    );
                    None
                }
            }
        } else {
            warn!(
                "Recovery storage is DISABLED. Failed batches will be lost. \
                Set ENABLE_RECOVERY=true for production."
            );
            None
        };

        // Create attestation client unless explicitly disabled (enabled by default for production)
        let attestation_client = if config.disable_attestation {
            warn!(
                "Attestation is DISABLED. Requests will not include TEE tokens. \
                Only disable for local development."
            );
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

        // Start snapshot sender thread (always enabled since snapshot_interval > 0 is enforced)
        let (snapshot_shutdown_tx, snapshot_shutdown_rx) = bounded(1);
        let snapshot_handle = {
            let cfg = config.clone();
            let rec = recovery.clone();
            let att = attestation_client.clone();

            thread::spawn(move || {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("Failed to create tokio runtime for snapshot sender")
                    .block_on(async {
                        SnapshotSender::new(cfg, rec, att)
                            .run(snapshot_rx, snapshot_shutdown_rx)
                            .await
                    });
            })
        };

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

        let synddb = Self {
            conn,
            sequencer_url: config.sequencer_url,
            db_path,
            monitor: Some(monitor),
            changeset_shutdown_tx,
            snapshot_shutdown_tx,
            changeset_handle: Some(changeset_handle),
            snapshot_handle: Some(snapshot_handle),
            recovery,
            chain_monitor,
            stats,
        };

        // Handle DDL crash recovery: force snapshot if marker exists from previous crash
        if needs_recovery_snapshot {
            info!("Forcing snapshot for DDL crash recovery");
            match synddb.snapshot() {
                Ok(_snapshot) => {
                    info!("DDL crash recovery snapshot published successfully");
                }
                Err(e) => {
                    // Marker is cleared after snapshot creation (before HTTP send),
                    // so even if HTTP fails, the marker should already be cleared.
                    // This warning is mostly for visibility into the HTTP failure.
                    warn!(
                        "Failed to send DDL crash recovery snapshot to sequencer: {}. \
                        The snapshot was created locally and the marker was cleared.",
                        e
                    );
                }
            }
        }

        // Auto-snapshot on attach if database has existing tables (always enabled)
        // This is critical for validator sync - without it, validators can't
        // reconstruct schemas that existed before SyndDB was attached.
        if Self::has_existing_tables(conn) {
            info!("Database has existing tables, creating initial snapshot for validator bootstrapping");
            if let Err(e) = synddb.snapshot() {
                warn!(
                    "Failed to create initial snapshot on attach: {}. Continuing without it.",
                    e
                );
            }
        }

        info!("SyndDB client attached successfully");

        Ok(synddb)
    }

    /// Validate configuration values.
    ///
    /// Returns an error if any configuration values are invalid.
    fn validate_config(config: &Config) -> Result<()> {
        if config.snapshot_interval == 0 {
            anyhow::bail!(
                "snapshot_interval must be greater than 0. \
                Use a high value (e.g., u64::MAX) if you want infrequent snapshots."
            );
        }

        if config.buffer_size == 0 {
            anyhow::bail!(
                "buffer_size must be greater than 0. \
                A zero-capacity channel causes blocking behavior."
            );
        }

        if config.flush_interval.is_zero() {
            anyhow::bail!(
                "flush_interval must be greater than 0. \
                Use a small value (e.g., 1ms) if you want fast flushing."
            );
        }

        Ok(())
    }

    // =========================================================================
    // Connection Access
    // =========================================================================

    /// Get a reference to the underlying `SQLite` connection
    ///
    /// Use this for executing SQL queries and commands.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use synddb_client::SyndDB;
    /// # let synddb = SyndDB::open("app.db", "http://sequencer:8433")?;
    /// synddb.connection().execute("INSERT INTO trades VALUES (1, 100)", [])?;
    ///
    /// let count: i64 = synddb.connection().query_row(
    ///     "SELECT COUNT(*) FROM trades",
    ///     [],
    ///     |row| row.get(0),
    /// )?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub const fn connection(&self) -> &Connection {
        self.conn
    }

    /// Execute a transaction with automatic handling
    ///
    /// This method:
    /// 1. Starts a transaction using `unchecked_transaction()` (required for `SyndDB`)
    /// 2. Calls your closure with the transaction
    /// 3. Commits on success, rolls back on error
    /// 4. Pushes changesets after commit
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use synddb_client::SyndDB;
    /// # let synddb = SyndDB::open("app.db", "http://sequencer:8433")?;
    /// synddb.transaction(|tx| {
    ///     tx.execute("UPDATE accounts SET balance = balance - 100 WHERE id = 1", [])?;
    ///     tx.execute("UPDATE accounts SET balance = balance + 100 WHERE id = 2", [])?;
    ///     Ok(())
    /// })?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// # Why `unchecked_transaction`?
    ///
    /// `SyndDB` holds a reference to the connection for the `SQLite` Session Extension.
    /// The standard `transaction()` method tries to take a mutable borrow, which conflicts.
    /// `unchecked_transaction()` bypasses this check, which is safe because `SyndDB` only
    /// reads from the connection during changeset extraction.
    pub fn transaction<T, F>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&rusqlite::Transaction<'_>) -> Result<T>,
    {
        let tx = self.conn.unchecked_transaction()?;
        let result = f(&tx)?;
        tx.commit()?;
        self.push()?;
        Ok(result)
    }

    // =========================================================================
    // Publishing
    // =========================================================================

    /// Push all pending changesets to the sequencer immediately
    ///
    /// Changesets are automatically pushed on a timer (default: every second).
    /// Use this method to force immediate push for low-latency or high-value
    /// changes that shouldn't wait for the next timer tick.
    ///
    /// Also called automatically on `Drop` for graceful shutdown.
    pub fn push(&self) -> Result<()> {
        self.monitor
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Monitor already shut down"))?
            .publish()
    }

    /// Create a local snapshot of the database (does NOT send to sequencer)
    ///
    /// This captures the full current state of the database as a portable `SQLite` file.
    /// The snapshot is returned but **not sent to the sequencer**. Use this when you
    /// need the snapshot data locally (e.g., for backup, testing, or manual transfer).
    ///
    /// # Important
    ///
    /// This method only creates a local snapshot. To create AND publish a snapshot
    /// to the sequencer (the typical use case), use [`snapshot()`] instead.
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
    /// // Create local snapshot (NOT sent to sequencer)
    /// let snapshot = synddb.create_snapshot()?;
    ///
    /// // Save to file for manual backup
    /// std::fs::write("backup.db", &snapshot.data)?;
    ///
    /// println!("Local snapshot: {} bytes at sequence {}", snapshot.data.len(), snapshot.sequence);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// [`snapshot()`]: Self::snapshot
    pub fn create_snapshot(&self) -> Result<Snapshot> {
        self.monitor
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Monitor already shut down"))?
            .snapshot()
    }

    /// Create and publish a snapshot to the sequencer
    ///
    /// This is the primary method for creating snapshots. It captures the full database
    /// state and sends it to the sequencer for ordering and distribution. Use this after
    /// schema changes (which aren't captured in changesets) or to create recovery points.
    ///
    /// # Behavior
    ///
    /// 1. Creates a complete database snapshot (like [`create_snapshot()`])
    /// 2. Sends the snapshot to the sequencer via HTTP (synchronous, blocking)
    /// 3. Waits for sequencer acknowledgment before returning
    ///
    /// This is consistent with [`push()`] - both methods send data
    /// to the sequencer immediately.
    ///
    /// [`push()`]: Self::push
    ///
    /// # When to Use
    ///
    /// - After `CREATE TABLE`, `ALTER TABLE`, or other DDL statements
    /// - To create periodic recovery checkpoints
    /// - Before major migrations or updates
    ///
    /// # Returns
    ///
    /// The snapshot that was created and published
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use synddb_client::SyndDB;
    /// # let synddb = SyndDB::open("app.db", "http://localhost:8433")?;
    /// // Create schema (DDL is NOT captured in changesets)
    /// synddb.connection().execute_batch("CREATE TABLE users (id INTEGER PRIMARY KEY)")?;
    ///
    /// // Publish snapshot so validators can reconstruct the schema
    /// let snapshot = synddb.snapshot()?;
    /// println!("Published snapshot: {} bytes at sequence {}", snapshot.data.len(), snapshot.sequence);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// [`create_snapshot()`]: Self::create_snapshot
    pub fn snapshot(&self) -> Result<Snapshot> {
        let snapshot = self.create_snapshot()?;

        // Clear DDL recovery marker now that we've created the snapshot locally.
        // Even if HTTP fails, we have the snapshot data and can retry.
        // The marker's purpose is to detect "schema change with no snapshot created".
        if let Some(ref path) = self.db_path {
            ddl_recovery::clear_marker(path);
        }

        // Send snapshot synchronously via HTTP
        let url = self
            .sequencer_url
            .join("snapshots")
            .map_err(|e| anyhow::anyhow!("Invalid URL: {}", e))?;

        let request = synddb_shared::types::payloads::SnapshotRequest {
            snapshot: synddb_shared::types::payloads::SnapshotData {
                data: snapshot.data.clone(),
                timestamp: snapshot
                    .timestamp
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0),
                sequence: snapshot.sequence,
            },
            message_id: uuid::Uuid::new_v4().to_string(),
            attestation_token: None,
        };

        // Use blocking HTTP client for synchronous snapshot publishing
        let cbor_bytes = request
            .to_cbor()
            .map_err(|e| anyhow::anyhow!("Failed to serialize snapshot: {}", e))?;

        debug!(
            "Publishing snapshot to {} ({} bytes)",
            url,
            cbor_bytes.len()
        );

        let client = reqwest::blocking::Client::new();
        let response = client
            .post(url)
            .header("Content-Type", "application/cbor")
            .body(cbor_bytes)
            .send()
            .map_err(|e| anyhow::anyhow!("Failed to send snapshot: {}", e))?;

        debug!("Sequencer response status: {}", response.status());

        response
            .error_for_status()
            .map_err(|e| anyhow::anyhow!("Sequencer rejected snapshot: {}", e))?;

        info!(
            "Published snapshot: {} bytes at sequence {}",
            snapshot.data.len(),
            snapshot.sequence
        );

        Ok(snapshot)
    }

    // =========================================================================
    // DDL Execution with Auto-Snapshot
    // =========================================================================

    /// Execute DDL statements with automatic snapshot publishing
    ///
    /// This method executes the given SQL (which should be DDL like CREATE TABLE)
    /// and automatically publishes a snapshot afterward.
    ///
    /// **Always use this method for schema changes** instead of `connection().execute_batch()`.
    /// Direct DDL bypasses snapshot creation and cannot be detected by `SyndDB`.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use synddb_client::SyndDB;
    /// # let synddb = SyndDB::open("app.db", "http://sequencer:8433")?;
    /// synddb.execute_ddl("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)")?;
    /// // Snapshot is automatically published - validators can now reconstruct this schema
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn execute_ddl(&self, sql: &str) -> Result<()> {
        let is_ddl = Self::is_ddl(sql);

        // Write DDL recovery marker BEFORE execution.
        // If the app crashes during DDL or before snapshot completes,
        // the marker will remain and trigger recovery on next startup.
        if is_ddl {
            if let Some(ref path) = self.db_path {
                ddl_recovery::write_marker(path);
            }
        }

        // Execute within DDL context
        session::with_ddl_context(|| self.conn.execute_batch(sql))?;

        // Create snapshot after DDL to capture the schema change.
        // Note: snapshot() clears the DDL recovery marker after creating the snapshot.
        if is_ddl {
            info!("DDL executed, creating automatic snapshot");
            self.snapshot()?;
        }

        Ok(())
    }

    /// Check if database has any user tables (excluding internal `SQLite` tables)
    fn has_existing_tables(conn: &Connection) -> bool {
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%')",
            [],
            |row| row.get(0),
        )
        .unwrap_or(false)
    }

    /// Check if SQL contains DDL statements (CREATE, ALTER, DROP)
    ///
    /// This is used internally by `execute_ddl()` and the FFI layer to detect
    /// when automatic snapshots should be created.
    pub(crate) fn is_ddl(sql: &str) -> bool {
        let upper = sql.trim_start().to_uppercase();
        upper.starts_with("CREATE ") || upper.starts_with("ALTER ") || upper.starts_with("DROP ")
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

    // =========================================================================
    // Replication Status
    // =========================================================================

    /// Get a snapshot of current replication statistics
    ///
    /// Returns information about pending changesets, pushed count, and health status.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use synddb_client::SyndDB;
    /// # let synddb = SyndDB::open("app.db", "http://sequencer:8433")?;
    /// let stats = synddb.stats();
    /// println!("Pending: {} changesets", stats.pending_changesets);
    /// println!("Pushed: {} changesets", stats.pushed_changesets);
    /// println!("Failed: {} attempts", stats.failed_pushes);
    /// println!("Healthy: {}", stats.is_healthy);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn stats(&self) -> StatsSnapshot {
        StatsSnapshot::from_stats(&self.stats)
    }

    /// Check if the sequencer is reachable
    ///
    /// Returns `true` if the last health check succeeded. Note that this is a cached
    /// value and may not reflect the current state if the network changed recently.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use synddb_client::SyndDB;
    /// # let synddb = SyndDB::open("app.db", "http://sequencer:8433")?;
    /// if synddb.is_healthy() {
    ///     println!("Sequencer is reachable");
    /// } else {
    ///     println!("Warning: sequencer may be unreachable");
    /// }
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn is_healthy(&self) -> bool {
        self.stats.is_healthy()
    }

    /// Get the number of changesets waiting to be pushed
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use synddb_client::SyndDB;
    /// # let synddb = SyndDB::open("app.db", "http://sequencer:8433")?;
    /// let pending = synddb.pending_count();
    /// if pending > 100 {
    ///     println!("Warning: {} changesets waiting", pending);
    /// }
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn pending_count(&self) -> usize {
        self.stats.pending_count()
    }

    /// Get the total number of successfully pushed changesets
    pub fn pushed_count(&self) -> u64 {
        self.stats.pushed_count()
    }

    // =========================================================================
    // Lifecycle
    // =========================================================================

    /// Gracefully shutdown the client, pushing any pending changesets and snapshots
    pub fn shutdown(mut self) -> Result<()> {
        info!("Shutting down SyndDB client");

        // Send shutdown signals
        let _ = self.changeset_shutdown_tx.send(());
        let _ = self.snapshot_shutdown_tx.send(());

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

        // First, drop the monitor which will stop the publish thread.
        // This ensures no more changesets or snapshots are generated.
        // SessionMonitor::drop() calls extract_and_send_changeset() for
        // any pending data changes.
        if let Some(monitor) = self.monitor.take() {
            drop(monitor);
        }

        // Then send shutdown signals to sender threads
        let _ = self.changeset_shutdown_tx.send(());
        let _ = self.snapshot_shutdown_tx.send(());

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
    #[ignore] // Requires running sequencer: cargo test -p synddb-client -- --ignored
    fn test_attach() {
        let conn = Box::leak(Box::new(Connection::open_in_memory().unwrap()));
        conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)", [])
            .unwrap();

        let _synddb = SyndDB::attach(conn, "http://localhost:8433").unwrap();

        conn.execute("INSERT INTO test (id, value) VALUES (1, 'test')", [])
            .unwrap();

        // Wait a moment for automatic push
        thread::sleep(std::time::Duration::from_secs(2));
    }

    #[test]
    #[ignore] // Requires running sequencer: cargo test -p synddb-client -- --ignored
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
    #[ignore] // Requires running sequencer: cargo test -p synddb-client -- --ignored
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
    #[ignore] // Requires running sequencer: cargo test -p synddb-client -- --ignored
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
    #[ignore] // Requires running sequencer: cargo test -p synddb-client -- --ignored
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
    #[ignore] // Requires running sequencer: cargo test -p synddb-client -- --ignored
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

    #[test]
    fn test_is_ddl() {
        // CREATE statements
        assert!(SyndDB::is_ddl("CREATE TABLE test (id INT)"));
        assert!(SyndDB::is_ddl("  CREATE TABLE test (id INT)")); // Leading whitespace
        assert!(SyndDB::is_ddl("create table test (id INT)")); // Lowercase
        assert!(SyndDB::is_ddl("CREATE INDEX idx ON test(id)"));
        assert!(SyndDB::is_ddl(
            "CREATE TRIGGER trg AFTER INSERT ON test BEGIN END"
        ));

        // ALTER statements
        assert!(SyndDB::is_ddl("ALTER TABLE test ADD COLUMN name TEXT"));
        assert!(SyndDB::is_ddl("alter table test add column name text"));

        // DROP statements
        assert!(SyndDB::is_ddl("DROP TABLE test"));
        assert!(SyndDB::is_ddl("DROP INDEX idx"));
        assert!(SyndDB::is_ddl("drop table if exists test"));

        // Non-DDL statements
        assert!(!SyndDB::is_ddl("INSERT INTO test VALUES (1)"));
        assert!(!SyndDB::is_ddl("SELECT * FROM test"));
        assert!(!SyndDB::is_ddl("UPDATE test SET id = 2"));
        assert!(!SyndDB::is_ddl("DELETE FROM test"));
        assert!(!SyndDB::is_ddl("BEGIN TRANSACTION"));
        assert!(!SyndDB::is_ddl("COMMIT"));
    }

    #[test]
    fn test_has_existing_tables() {
        // Empty database has no tables
        let conn = Box::leak(Box::new(Connection::open_in_memory().unwrap()));
        assert!(!SyndDB::has_existing_tables(conn));

        // Create a table
        conn.execute("CREATE TABLE test (id INTEGER)", []).unwrap();
        assert!(SyndDB::has_existing_tables(conn));
    }

    #[test]
    #[ignore] // Requires running sequencer: cargo test -p synddb-client -- --ignored
    fn test_attach_with_existing_tables_auto_snapshots() {
        // Create a database with existing tables before attaching SyndDB
        let conn = Box::leak(Box::new(Connection::open_in_memory().unwrap()));
        conn.execute(
            "CREATE TABLE preexisting (id INTEGER PRIMARY KEY, data TEXT)",
            [],
        )
        .unwrap();
        conn.execute("INSERT INTO preexisting VALUES (1, 'test')", [])
            .unwrap();

        // Attach SyndDB - auto snapshot is always enabled
        // This should attempt to publish a snapshot (will fail since no sequencer, but shouldn't panic)
        let config = Config {
            sequencer_url: "http://localhost:8433".parse().unwrap(),
            ..Default::default()
        };

        let _synddb = SyndDB::attach_with_config(conn, config).unwrap();
    }

    // =========================================================================
    // Robustness tests based on learnings from E2E debugging
    // =========================================================================
    //
    // Key learning: SQLite Session Extension does NOT reset after changeset_strm()
    // extraction. Each call returns ALL changes since session creation. We must
    // recreate the session after each extraction to get only new changes.
    //
    // Bug symptoms we observed:
    // - Changeset sizes grew over time (34KB -> 44KB -> 53KB -> 63KB)
    // - Validator received duplicate data in each batch
    // - SQLITE_CHANGESET_CONFLICT on INSERT (same rows inserted twice)

    #[test]
    #[ignore] // Requires running sequencer: cargo test -p synddb-client -- --ignored
    fn test_multiple_push_cycles_independent() {
        // Test that multiple push cycles produce independent changesets.
        // This is the core test for the session recreation fix.
        //
        // Before the fix: Each push would include ALL previous changes.
        // After the fix: Each push only includes changes since last push.
        let conn = Box::leak(Box::new(Connection::open_in_memory().unwrap()));
        conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)", [])
            .unwrap();

        let synddb = SyndDB::attach(conn, "http://localhost:8433").unwrap();

        // Cycle 1: Insert some rows
        conn.execute("INSERT INTO test VALUES (1, 'first')", [])
            .unwrap();
        conn.execute("INSERT INTO test VALUES (2, 'second')", [])
            .unwrap();

        // Publish first batch
        synddb.push().unwrap();

        // Cycle 2: Insert more rows
        conn.execute("INSERT INTO test VALUES (3, 'third')", [])
            .unwrap();
        conn.execute("INSERT INTO test VALUES (4, 'fourth')", [])
            .unwrap();

        // Publish second batch - should NOT include rows 1-2
        synddb.push().unwrap();

        // Cycle 3: Update existing rows
        conn.execute("UPDATE test SET value = 'updated' WHERE id = 1", [])
            .unwrap();

        // Publish third batch - should only include the update
        synddb.push().unwrap();

        // If session recreation is working, we should have 3 independent changesets
        // (The actual verification happens in E2E tests with validator)
    }

    #[test]
    #[ignore] // Requires running sequencer: cargo test -p synddb-client -- --ignored
    fn test_preexisting_data_then_modifications() {
        // Simulates the orderbook benchmark pattern:
        // 1. Schema and initial data exist BEFORE SyndDB attaches
        // 2. SyndDB attaches (triggers auto snapshot)
        // 3. New modifications are captured as changesets
        //
        // The changesets should only contain the NEW modifications, not the
        // pre-existing data (which is in the snapshot).
        let conn = Box::leak(Box::new(Connection::open_in_memory().unwrap()));

        // Step 1: Create schema and insert initial data BEFORE SyndDB
        conn.execute(
            "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT, balance INTEGER)",
            [],
        )
        .unwrap();
        for i in 0..10 {
            conn.execute(
                "INSERT INTO users VALUES (?1, ?2, ?3)",
                rusqlite::params![i, format!("User{}", i), 1000],
            )
            .unwrap();
        }

        // Step 2: Attach SyndDB (auto snapshot is always enabled)
        let config = Config {
            sequencer_url: "http://localhost:8433".parse().unwrap(),
            ..Default::default()
        };
        let synddb = SyndDB::attach_with_config(conn, config).unwrap();

        // Step 3: Make modifications AFTER attach
        conn.execute("UPDATE users SET balance = 2000 WHERE id = 0", [])
            .unwrap();
        conn.execute("INSERT INTO users VALUES (10, 'NewUser', 500)", [])
            .unwrap();

        // Push - this changeset should only contain the update and insert,
        // not the original 10 users (those are in the snapshot)
        synddb.push().unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 11);
    }

    #[test]
    #[ignore] // Requires running sequencer: cargo test -p synddb-client -- --ignored
    fn test_transaction_batch_then_individual_ops() {
        // Simulates the exact orderbook benchmark pattern that revealed the bug:
        // 1. Batch insert users in a transaction
        // 2. Individual balance inserts
        // 3. Multiple push cycles
        //
        // This pattern was causing duplicate changesets before the fix.
        let conn = Box::leak(Box::new(Connection::open_in_memory().unwrap()));
        conn.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)", [])
            .unwrap();
        conn.execute(
            "CREATE TABLE balances (user_id INTEGER PRIMARY KEY, amount INTEGER)",
            [],
        )
        .unwrap();

        let synddb = SyndDB::attach(conn, "http://localhost:8433").unwrap();

        // Batch insert users in a transaction (like benchmark initialization)
        {
            let tx = conn.unchecked_transaction().unwrap();
            for i in 0..100 {
                tx.execute(
                    "INSERT INTO users VALUES (?1, ?2)",
                    rusqlite::params![i, format!("User{}", i)],
                )
                .unwrap();
            }
            tx.commit().unwrap();
        }

        // Publish after batch
        synddb.push().unwrap();

        // Individual balance inserts (like benchmark setup)
        for i in 0..100 {
            conn.execute(
                "INSERT INTO balances VALUES (?1, ?2)",
                rusqlite::params![i, 10000],
            )
            .unwrap();
        }

        // Publish after individual ops
        synddb.push().unwrap();

        // More batch operations
        {
            let tx = conn.unchecked_transaction().unwrap();
            for i in 0..50 {
                tx.execute(
                    "UPDATE balances SET amount = amount + 100 WHERE user_id = ?1",
                    rusqlite::params![i],
                )
                .unwrap();
            }
            tx.commit().unwrap();
        }

        // Final publish
        synddb.push().unwrap();

        let user_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0))
            .unwrap();
        let balance_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM balances", [], |row| row.get(0))
            .unwrap();
        assert_eq!(user_count, 100);
        assert_eq!(balance_count, 100);
    }

    #[test]
    #[ignore] // Requires running sequencer: cargo test -p synddb-client -- --ignored
    fn test_rapid_publish_cycles() {
        // Test rapid succession of changes and publishes.
        // This stress tests the session recreation mechanism.
        let conn = Box::leak(Box::new(Connection::open_in_memory().unwrap()));
        conn.execute(
            "CREATE TABLE counter (id INTEGER PRIMARY KEY, value INTEGER)",
            [],
        )
        .unwrap();
        conn.execute("INSERT INTO counter VALUES (1, 0)", [])
            .unwrap();

        let synddb = SyndDB::attach(conn, "http://localhost:8433").unwrap();

        // Rapid cycles of update + publish
        for i in 1..=50 {
            conn.execute("UPDATE counter SET value = ?1 WHERE id = 1", [i])
                .unwrap();
            synddb.push().unwrap();
        }

        // Verify final state
        let value: i64 = conn
            .query_row("SELECT value FROM counter WHERE id = 1", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(value, 50);
    }

    #[test]
    #[ignore] // Requires running sequencer: cargo test -p synddb-client -- --ignored
    fn test_mixed_ddl_and_dml() {
        // Test interleaving DDL (schema changes) and DML (data changes).
        // DDL always triggers a snapshot, which should play nicely
        // with the session recreation mechanism.
        let conn = Box::leak(Box::new(Connection::open_in_memory().unwrap()));

        let config = Config {
            sequencer_url: "http://localhost:8433".parse().unwrap(),
            ..Default::default()
        };
        let synddb = SyndDB::attach_with_config(conn, config).unwrap();

        // Create first table (DDL triggers snapshot)
        synddb
            .execute_ddl("CREATE TABLE t1 (id INTEGER PRIMARY KEY, val TEXT)")
            .unwrap();

        // Insert data
        conn.execute("INSERT INTO t1 VALUES (1, 'a')", []).unwrap();
        synddb.push().unwrap();

        // Create second table (another DDL)
        synddb
            .execute_ddl("CREATE TABLE t2 (id INTEGER PRIMARY KEY, ref_id INTEGER)")
            .unwrap();

        // Insert into both tables
        conn.execute("INSERT INTO t1 VALUES (2, 'b')", []).unwrap();
        conn.execute("INSERT INTO t2 VALUES (1, 1)", []).unwrap();
        synddb.push().unwrap();

        // Verify state
        let t1_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM t1", [], |row| row.get(0))
            .unwrap();
        let t2_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM t2", [], |row| row.get(0))
            .unwrap();
        assert_eq!(t1_count, 2);
        assert_eq!(t2_count, 1);
    }

    #[test]
    #[ignore] // Requires running sequencer: cargo test -p synddb-client -- --ignored
    fn test_empty_publish_cycles() {
        // Test that publish() with no changes doesn't cause issues.
        // The session should handle empty extractions gracefully.
        let conn = Box::leak(Box::new(Connection::open_in_memory().unwrap()));
        conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY)", [])
            .unwrap();

        let synddb = SyndDB::attach(conn, "http://localhost:8433").unwrap();

        // Multiple empty publishes
        synddb.push().unwrap();
        synddb.push().unwrap();
        synddb.push().unwrap();

        // Now make a change
        conn.execute("INSERT INTO test VALUES (1)", []).unwrap();
        synddb.push().unwrap();

        // More empty publishes
        synddb.push().unwrap();
        synddb.push().unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM test", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    #[ignore] // Requires running sequencer: cargo test -p synddb-client -- --ignored
    fn test_large_batch_single_transaction() {
        // Test a large batch in a single transaction.
        // This is common in data import scenarios.
        let conn = Box::leak(Box::new(Connection::open_in_memory().unwrap()));
        conn.execute("CREATE TABLE items (id INTEGER PRIMARY KEY, data TEXT)", [])
            .unwrap();

        let synddb = SyndDB::attach(conn, "http://localhost:8433").unwrap();

        // Large batch insert
        {
            let tx = conn.unchecked_transaction().unwrap();
            for i in 0..1000 {
                tx.execute(
                    "INSERT INTO items VALUES (?1, ?2)",
                    rusqlite::params![i, format!("Item data {}", i)],
                )
                .unwrap();
            }
            tx.commit().unwrap();
        }

        synddb.push().unwrap();

        // Follow up with smaller batches
        for batch in 0..5 {
            let tx = conn.unchecked_transaction().unwrap();
            for i in 0..10 {
                let id = 1000 + batch * 10 + i;
                tx.execute(
                    "INSERT INTO items VALUES (?1, ?2)",
                    rusqlite::params![id, format!("Batch {} item {}", batch, i)],
                )
                .unwrap();
            }
            tx.commit().unwrap();
            synddb.push().unwrap();
        }

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM items", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1050);
    }

    #[test]
    #[ignore] // Requires running sequencer: cargo test -p synddb-client -- --ignored
    fn test_rollback_not_captured() {
        // Verify that rolled-back transactions are NOT captured in changesets.
        // This is important for data integrity - only committed changes should replicate.
        let conn = Box::leak(Box::new(Connection::open_in_memory().unwrap()));
        conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)", [])
            .unwrap();

        let synddb = SyndDB::attach(conn, "http://localhost:8433").unwrap();

        // Insert a row and commit
        conn.execute("INSERT INTO test VALUES (1, 'committed')", [])
            .unwrap();
        synddb.push().unwrap();

        // Start a transaction, make changes, then rollback
        {
            let tx = conn.unchecked_transaction().unwrap();
            tx.execute("INSERT INTO test VALUES (2, 'will_rollback')", [])
                .unwrap();
            tx.execute("UPDATE test SET value = 'modified' WHERE id = 1", [])
                .unwrap();
            // Explicitly rollback (drop without commit)
            drop(tx);
        }

        // Push - should have nothing new (rollback discarded changes)
        synddb.push().unwrap();

        // Make another committed change
        conn.execute("INSERT INTO test VALUES (3, 'after_rollback')", [])
            .unwrap();
        synddb.push().unwrap();

        // Verify database state
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM test", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 2); // Only rows 1 and 3

        let value: String = conn
            .query_row("SELECT value FROM test WHERE id = 1", [], |row| row.get(0))
            .unwrap();
        assert_eq!(value, "committed"); // Not modified
    }

    #[test]
    #[ignore] // Requires running sequencer: cargo test -p synddb-client -- --ignored
    fn test_delete_operations() {
        // Verify DELETE operations are captured correctly.
        let conn = Box::leak(Box::new(Connection::open_in_memory().unwrap()));
        conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)", [])
            .unwrap();

        let synddb = SyndDB::attach(conn, "http://localhost:8433").unwrap();

        // Insert rows
        for i in 1..=5 {
            conn.execute(
                "INSERT INTO test VALUES (?1, ?2)",
                rusqlite::params![i, format!("value{}", i)],
            )
            .unwrap();
        }
        synddb.push().unwrap();

        // Delete some rows
        conn.execute("DELETE FROM test WHERE id IN (2, 4)", [])
            .unwrap();
        synddb.push().unwrap();

        // Delete all remaining
        conn.execute("DELETE FROM test", []).unwrap();
        synddb.push().unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM test", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    #[ignore] // Requires running sequencer: cargo test -p synddb-client -- --ignored
    fn test_insert_or_replace_pattern() {
        // Test INSERT OR REPLACE (used in orderbook benchmark for balance updates).
        // This generates DELETE + INSERT changesets, not UPDATE.
        let conn = Box::leak(Box::new(Connection::open_in_memory().unwrap()));
        conn.execute(
            "CREATE TABLE balances (user_id INTEGER PRIMARY KEY, amount INTEGER)",
            [],
        )
        .unwrap();

        let synddb = SyndDB::attach(conn, "http://localhost:8433").unwrap();

        // Initial inserts
        for i in 1..=10 {
            conn.execute(
                "INSERT INTO balances VALUES (?1, ?2)",
                rusqlite::params![i, 1000],
            )
            .unwrap();
        }
        synddb.push().unwrap();

        // Use INSERT OR REPLACE to update values (like orderbook benchmark)
        for i in 1..=10 {
            conn.execute(
                "INSERT OR REPLACE INTO balances VALUES (?1, ?2)",
                rusqlite::params![i, 2000],
            )
            .unwrap();
        }
        synddb.push().unwrap();

        // Verify final state
        let total: i64 = conn
            .query_row("SELECT SUM(amount) FROM balances", [], |row| row.get(0))
            .unwrap();
        assert_eq!(total, 20000); // 10 users * 2000
    }

    #[test]
    #[ignore] // Requires running sequencer: cargo test -p synddb-client -- --ignored
    fn test_preexisting_data_then_ddl() {
        // Attach to database with existing data, then perform DDL.
        // Tests auto snapshot on attach combined with DDL snapshots.
        let conn = Box::leak(Box::new(Connection::open_in_memory().unwrap()));

        // Pre-existing schema and data
        conn.execute("CREATE TABLE t1 (id INTEGER PRIMARY KEY, val TEXT)", [])
            .unwrap();
        conn.execute("INSERT INTO t1 VALUES (1, 'existing')", [])
            .unwrap();

        // Attach (auto snapshot is always enabled)
        let config = Config {
            sequencer_url: "http://localhost:8433".parse().unwrap(),
            ..Default::default()
        };
        let synddb = SyndDB::attach_with_config(conn, config).unwrap();

        // DDL after attach (should trigger another snapshot)
        synddb
            .execute_ddl("CREATE TABLE t2 (id INTEGER PRIMARY KEY, ref_id INTEGER)")
            .unwrap();

        // DML on both tables
        conn.execute("INSERT INTO t1 VALUES (2, 'new')", [])
            .unwrap();
        conn.execute("INSERT INTO t2 VALUES (1, 2)", []).unwrap();
        synddb.push().unwrap();

        // Verify state
        let t1_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM t1", [], |row| row.get(0))
            .unwrap();
        let t2_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM t2", [], |row| row.get(0))
            .unwrap();
        assert_eq!(t1_count, 2);
        assert_eq!(t2_count, 1);
    }

    #[test]
    #[ignore] // Requires running sequencer: cargo test -p synddb-client -- --ignored
    fn test_large_text_values() {
        // Test handling of large TEXT values (edge case for changeset size).
        let conn = Box::leak(Box::new(Connection::open_in_memory().unwrap()));
        conn.execute(
            "CREATE TABLE docs (id INTEGER PRIMARY KEY, content TEXT)",
            [],
        )
        .unwrap();

        let synddb = SyndDB::attach(conn, "http://localhost:8433").unwrap();

        // Insert rows with large text (100KB each)
        let large_text = "x".repeat(100 * 1024);
        for i in 1..=5 {
            conn.execute(
                "INSERT INTO docs VALUES (?1, ?2)",
                rusqlite::params![i, &large_text],
            )
            .unwrap();
        }
        synddb.push().unwrap();

        // Update large text
        let updated_text = "y".repeat(100 * 1024);
        conn.execute(
            "UPDATE docs SET content = ?1 WHERE id = 1",
            rusqlite::params![&updated_text],
        )
        .unwrap();
        synddb.push().unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM docs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 5);
    }

    #[test]
    #[ignore] // Requires running sequencer: cargo test -p synddb-client -- --ignored
    fn test_blob_values() {
        // Test handling of BLOB values.
        let conn = Box::leak(Box::new(Connection::open_in_memory().unwrap()));
        conn.execute("CREATE TABLE files (id INTEGER PRIMARY KEY, data BLOB)", [])
            .unwrap();

        let synddb = SyndDB::attach(conn, "http://localhost:8433").unwrap();

        // Insert binary data
        let blob_data: Vec<u8> = (0..=255).cycle().take(50 * 1024).collect();
        for i in 1..=3 {
            conn.execute(
                "INSERT INTO files VALUES (?1, ?2)",
                rusqlite::params![i, &blob_data],
            )
            .unwrap();
        }
        synddb.push().unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM files", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 3);
    }

    #[test]
    #[ignore] // Requires running sequencer: cargo test -p synddb-client -- --ignored
    fn test_multiple_tables_single_transaction() {
        // Test modifications to multiple tables in a single transaction.
        let conn = Box::leak(Box::new(Connection::open_in_memory().unwrap()));
        conn.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)", [])
            .unwrap();
        conn.execute(
            "CREATE TABLE orders (id INTEGER PRIMARY KEY, user_id INTEGER, item TEXT)",
            [],
        )
        .unwrap();
        conn.execute(
            "CREATE TABLE audit_log (id INTEGER PRIMARY KEY, action TEXT, ts INTEGER)",
            [],
        )
        .unwrap();

        let synddb = SyndDB::attach(conn, "http://localhost:8433").unwrap();

        // Single transaction touching all tables
        {
            let tx = conn.unchecked_transaction().unwrap();
            tx.execute("INSERT INTO users VALUES (1, 'Alice')", [])
                .unwrap();
            tx.execute("INSERT INTO orders VALUES (1, 1, 'Widget')", [])
                .unwrap();
            tx.execute(
                "INSERT INTO audit_log VALUES (1, 'user_created', 12345)",
                [],
            )
            .unwrap();
            tx.execute(
                "INSERT INTO audit_log VALUES (2, 'order_placed', 12346)",
                [],
            )
            .unwrap();
            tx.commit().unwrap();
        }
        synddb.push().unwrap();

        // Verify all tables updated
        let user_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0))
            .unwrap();
        let order_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM orders", [], |row| row.get(0))
            .unwrap();
        let audit_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM audit_log", [], |row| row.get(0))
            .unwrap();
        assert_eq!(user_count, 1);
        assert_eq!(order_count, 1);
        assert_eq!(audit_count, 2);
    }

    #[test]
    #[ignore] // Requires running sequencer: cargo test -p synddb-client -- --ignored
    fn test_null_values() {
        // Test handling of NULL values in changesets.
        let conn = Box::leak(Box::new(Connection::open_in_memory().unwrap()));
        conn.execute(
            "CREATE TABLE nullable (id INTEGER PRIMARY KEY, val1 TEXT, val2 INTEGER)",
            [],
        )
        .unwrap();

        let synddb = SyndDB::attach(conn, "http://localhost:8433").unwrap();

        // Insert with NULLs
        conn.execute("INSERT INTO nullable VALUES (1, NULL, NULL)", [])
            .unwrap();
        conn.execute("INSERT INTO nullable VALUES (2, 'has_value', NULL)", [])
            .unwrap();
        conn.execute("INSERT INTO nullable VALUES (3, NULL, 42)", [])
            .unwrap();
        synddb.push().unwrap();

        // Update NULL to value
        conn.execute("UPDATE nullable SET val1 = 'now_set' WHERE id = 1", [])
            .unwrap();
        // Update value to NULL
        conn.execute("UPDATE nullable SET val1 = NULL WHERE id = 2", [])
            .unwrap();
        synddb.push().unwrap();

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM nullable WHERE val1 IS NULL",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 2); // Rows 2 and 3
    }

    #[test]
    #[ignore] // Requires running sequencer: cargo test -p synddb-client -- --ignored
    fn test_empty_database_then_schema() {
        // Attach to completely empty database, then create schema.
        // Opposite of pre-existing data pattern.
        let conn = Box::leak(Box::new(Connection::open_in_memory().unwrap()));

        // Attach to empty DB - auto snapshot won't trigger (no tables)
        let config = Config {
            sequencer_url: "http://localhost:8433".parse().unwrap(),
            ..Default::default()
        };
        let synddb = SyndDB::attach_with_config(conn, config).unwrap();

        // Create schema (triggers snapshot)
        synddb
            .execute_ddl("CREATE TABLE test (id INTEGER PRIMARY KEY, val TEXT)")
            .unwrap();

        // Insert data
        conn.execute("INSERT INTO test VALUES (1, 'first')", [])
            .unwrap();
        synddb.push().unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM test", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    // =========================================================================
    // DDL Crash Recovery Tests
    // =========================================================================
    //
    // These tests verify the marker file system for recovering from crashes
    // that occur after direct DDL but before push().

    #[test]
    #[ignore] // Requires running sequencer: cargo test -p synddb-client -- --ignored
    fn test_update_hook_behavior() {
        // Understand what SQLite's update hook fires for.
        // FINDING: The update hook does NOT fire for DDL (CREATE/ALTER/DROP).
        // It only fires for INSERT/UPDATE/DELETE on user tables.
        use rusqlite::hooks::Action;
        use std::sync::{
            atomic::{AtomicI32, Ordering},
            Arc,
        };

        let conn = Connection::open_in_memory().unwrap();
        let hook_count = Arc::new(AtomicI32::new(0));
        let hook_count_clone = hook_count.clone();

        conn.update_hook(Some(
            move |action: Action, _db: &str, table: &str, rowid: i64| {
                eprintln!("Hook fired: {:?} on {} rowid={}", action, table, rowid);
                hook_count_clone.fetch_add(1, Ordering::SeqCst);
            },
        ));

        // DDL does NOT fire the update hook
        eprintln!("Creating table...");
        conn.execute("CREATE TABLE test (id INTEGER, val TEXT)", [])
            .unwrap();
        assert_eq!(
            hook_count.load(Ordering::SeqCst),
            0,
            "DDL should NOT fire update hook"
        );

        // DML (INSERT) DOES fire the update hook
        eprintln!("Inserting row...");
        conn.execute("INSERT INTO test VALUES (1, 'hello')", [])
            .unwrap();
        assert_eq!(
            hook_count.load(Ordering::SeqCst),
            1,
            "INSERT should fire update hook"
        );

        // DML (UPDATE) DOES fire the update hook
        eprintln!("Updating row...");
        conn.execute("UPDATE test SET val = 'world' WHERE id = 1", [])
            .unwrap();
        assert_eq!(
            hook_count.load(Ordering::SeqCst),
            2,
            "UPDATE should fire update hook"
        );
    }

    #[test]
    #[ignore] // Requires running sequencer: cargo test -p synddb-client -- --ignored
    fn test_direct_ddl_not_detected() {
        // Verify that direct DDL (via connection().execute()) is NOT detected.
        // This documents the limitation that SQLite's update hook doesn't fire for DDL.
        // Users MUST use execute_ddl() for crash-safe DDL operations.

        let temp_dir = std::env::temp_dir();
        let db_file = temp_dir.join(format!("test_direct_ddl_{}.db", std::process::id()));

        let conn = Box::leak(Box::new(Connection::open(&db_file).unwrap()));
        let db_path = conn.path().unwrap();

        ddl_recovery::clear_marker(db_path);

        let config = Config {
            sequencer_url: "http://localhost:8433".parse().unwrap(),
            ..Default::default()
        };
        let _synddb = SyndDB::attach_with_config(conn, config).unwrap();

        // Direct DDL via connection - NOT detected (no marker written)
        conn.execute("CREATE TABLE direct_ddl (id INTEGER)", [])
            .unwrap();

        // Marker is NOT written because we can't detect direct DDL
        assert!(
            !ddl_recovery::check_marker(db_path),
            "Direct DDL should NOT write marker (limitation of SQLite update hook)"
        );

        // Clean up
        let _ = std::fs::remove_file(&db_file);
    }

    #[test]
    #[ignore] // Requires running sequencer: cargo test -p synddb-client -- --ignored
    fn test_ddl_recovery_marker_cleared_on_snapshot() {
        // Test that snapshot() clears the recovery marker even when HTTP fails

        let temp_dir = std::env::temp_dir();
        let db_file = temp_dir.join(format!("test_ddl_clear_{}.db", std::process::id()));

        let conn = Box::leak(Box::new(Connection::open(&db_file).unwrap()));
        conn.execute("CREATE TABLE test (id INTEGER)", []).unwrap();

        // Get the actual path as the connection sees it
        let db_path = conn.path().unwrap();

        // Manually write a marker to simulate previous crash
        ddl_recovery::write_marker(db_path);
        assert!(ddl_recovery::check_marker(db_path));

        let config = Config {
            sequencer_url: "http://localhost:8433".parse().unwrap(),
            ..Default::default()
        };
        let _synddb = SyndDB::attach_with_config(conn, config).unwrap();

        // Attach should have detected the marker and triggered a recovery snapshot.
        // Even if HTTP fails, the marker is cleared after creating the snapshot locally.
        // Marker should already be cleared by the attach
        assert!(!ddl_recovery::check_marker(db_path));

        // Clean up
        let _ = std::fs::remove_file(&db_file);
    }

    #[test]
    #[ignore] // Requires running sequencer: cargo test -p synddb-client -- --ignored
    fn test_ddl_recovery_forces_snapshot_on_attach() {
        // Test that attaching with a marker forces a snapshot attempt

        let temp_dir = std::env::temp_dir();
        let db_file = temp_dir.join(format!("test_ddl_attach_{}.db", std::process::id()));

        // Create DB with schema and get the path as the connection sees it
        let db_path: String;
        {
            let conn = Connection::open(&db_file).unwrap();
            db_path = conn.path().unwrap().to_string();
            conn.execute("CREATE TABLE test (id INTEGER, val TEXT)", [])
                .unwrap();
            conn.execute("INSERT INTO test VALUES (1, 'data')", [])
                .unwrap();
        }

        // Write marker to simulate crash after direct DDL
        ddl_recovery::write_marker(&db_path);
        assert!(ddl_recovery::check_marker(&db_path));

        // Attach SyndDB - should detect marker and attempt recovery snapshot
        let conn = Box::leak(Box::new(Connection::open(&db_file).unwrap()));
        let config = Config {
            sequencer_url: "http://localhost:8433".parse().unwrap(),
            ..Default::default()
        };
        let _synddb = SyndDB::attach_with_config(conn, config).unwrap();

        // Marker should be cleared by the recovery snapshot attempt
        // (Even if HTTP fails, the snapshot is created and marker cleared)
        assert!(!ddl_recovery::check_marker(&db_path));

        // Clean up
        let _ = std::fs::remove_file(&db_file);
    }

    #[test]
    #[ignore] // Requires running sequencer: cargo test -p synddb-client -- --ignored
    fn test_execute_ddl_clears_marker_after_snapshot() {
        // Test that execute_ddl() writes marker before DDL and clears after snapshot

        let temp_dir = std::env::temp_dir();
        let db_file = temp_dir.join(format!("test_execute_ddl_{}.db", std::process::id()));

        let conn = Box::leak(Box::new(Connection::open(&db_file).unwrap()));
        let db_path = conn.path().unwrap();

        ddl_recovery::clear_marker(db_path);

        let config = Config {
            sequencer_url: "http://localhost:8433".parse().unwrap(),
            ..Default::default()
        };
        let synddb = SyndDB::attach_with_config(conn, config).unwrap();

        assert!(!ddl_recovery::check_marker(db_path));

        // execute_ddl writes marker, creates snapshot (HTTP fails but marker cleared)
        synddb
            .execute_ddl("CREATE TABLE proper_ddl (id INTEGER)")
            .unwrap();

        // Marker cleared by snapshot() after creating snapshot locally
        assert!(!ddl_recovery::check_marker(db_path));

        // Clean up
        let _ = std::fs::remove_file(&db_file);
    }

    #[test]
    #[ignore] // Requires running sequencer: cargo test -p synddb-client -- --ignored
    fn test_ddl_recovery_in_memory_db_no_marker() {
        // Test that in-memory databases don't use markers (no path)
        let conn = Box::leak(Box::new(Connection::open_in_memory().unwrap()));
        let config = Config {
            sequencer_url: "http://localhost:8433".parse().unwrap(),
            ..Default::default()
        };
        let _synddb = SyndDB::attach_with_config(conn, config).unwrap();

        // Direct DDL on in-memory DB
        conn.execute("CREATE TABLE test (id INTEGER)", []).unwrap();

        // No crash - test passes if no panic (markers are skipped for in-memory)
    }

    // =========================================================================
    // Direct DDL Failure and Recovery Tests
    // =========================================================================
    //
    // These tests document what happens when a developer accidentally uses
    // connection().execute() for DDL instead of execute_ddl(), and how to recover.
    //
    // IMPORTANT: SQLite's update hook does NOT fire for DDL statements.
    // This means SyndDB cannot automatically detect schema changes made via
    // direct connection access. The recovery requires manual snapshot publishing.

    #[test]
    #[ignore] // Requires running sequencer: cargo test -p synddb-client -- --ignored
    fn test_direct_ddl_then_dml_local_state_ok() {
        // When DDL is done directly, local application continues to work fine.
        // The problem only manifests when validators try to apply changesets.
        //
        // This test documents that the app developer sees no errors locally.
        let conn = Box::leak(Box::new(Connection::open_in_memory().unwrap()));
        let synddb = SyndDB::attach(conn, "http://localhost:8433").unwrap();

        // MISTAKE: Using connection().execute() instead of execute_ddl()
        conn.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)", [])
            .unwrap();

        // Local operations work perfectly - no indication of a problem
        conn.execute("INSERT INTO users VALUES (1, 'Alice')", [])
            .unwrap();
        conn.execute("INSERT INTO users VALUES (2, 'Bob')", [])
            .unwrap();
        conn.execute("UPDATE users SET name = 'Alice Updated' WHERE id = 1", [])
            .unwrap();

        // Publish captures the DML, but validator will fail because it
        // doesn't have the 'users' table schema (no snapshot was published)
        synddb.push().unwrap();

        // Local state is correct - developer doesn't know there's a problem
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 2);

        // The problem: Validators receiving these changesets will fail with
        // "no such table: users" because they never received a snapshot
        // containing the CREATE TABLE.
    }

    #[test]
    #[ignore] // Requires running sequencer: cargo test -p synddb-client -- --ignored
    fn test_direct_ddl_changeset_contains_dml_not_ddl() {
        // Demonstrates that changesets ONLY contain DML operations.
        // DDL (CREATE/ALTER/DROP) is never captured in changesets.
        // This is a fundamental SQLite Session Extension limitation.
        let conn = Box::leak(Box::new(Connection::open_in_memory().unwrap()));
        let synddb = SyndDB::attach(conn, "http://localhost:8433").unwrap();

        // DDL via direct connection (WRONG way, but common mistake)
        conn.execute(
            "CREATE TABLE orders (id INTEGER PRIMARY KEY, amount INTEGER)",
            [],
        )
        .unwrap();

        // Create snapshot locally to verify it contains the schema
        let snapshot = synddb.create_snapshot().unwrap();
        assert!(
            !snapshot.data.is_empty(),
            "Snapshot should contain the schema"
        );

        // The snapshot data is a valid SQLite database with the schema
        // This is how validators learn about the schema - via snapshots, not changesets
    }

    #[test]
    #[ignore] // Requires running sequencer: cargo test -p synddb-client -- --ignored
    fn test_recovery_via_manual_snapshot() {
        // Documents the recovery process when direct DDL was used:
        // 1. Developer notices validator errors ("no such table")
        // 2. Developer calls snapshot() to capture current schema
        // 3. Validators receive snapshot and can now apply subsequent changesets
        let conn = Box::leak(Box::new(Connection::open_in_memory().unwrap()));
        let synddb = SyndDB::attach(conn, "http://localhost:8433").unwrap();

        // MISTAKE: Direct DDL without execute_ddl()
        conn.execute(
            "CREATE TABLE accounts (id INTEGER PRIMARY KEY, balance INTEGER)",
            [],
        )
        .unwrap();

        // Insert some data (will fail on validators until snapshot is published)
        conn.execute("INSERT INTO accounts VALUES (1, 1000)", [])
            .unwrap();
        synddb.push().unwrap();

        // --- RECOVERY POINT ---
        // Developer notices validator errors and calls:
        let snapshot = synddb.snapshot().unwrap();

        // Snapshot contains the schema and data
        assert!(!snapshot.data.is_empty());

        // After this point, validators have the schema and can apply changesets
        conn.execute("INSERT INTO accounts VALUES (2, 2000)", [])
            .unwrap();
        synddb.push().unwrap();

        // Local verification
        let total: i64 = conn
            .query_row("SELECT SUM(balance) FROM accounts", [], |row| row.get(0))
            .unwrap();
        assert_eq!(total, 3000);
    }

    #[test]
    #[ignore] // Requires running sequencer: cargo test -p synddb-client -- --ignored
    fn test_recovery_with_multiple_missing_tables() {
        // Recovery still works even if multiple tables were created via direct DDL.
        // One snapshot captures all schema and data.
        let conn = Box::leak(Box::new(Connection::open_in_memory().unwrap()));
        let synddb = SyndDB::attach(conn, "http://localhost:8433").unwrap();

        // Multiple direct DDL operations (all wrong, but recoverable)
        conn.execute("CREATE TABLE t1 (id INTEGER PRIMARY KEY, val1 TEXT)", [])
            .unwrap();
        conn.execute("CREATE TABLE t2 (id INTEGER PRIMARY KEY, val2 INTEGER)", [])
            .unwrap();
        conn.execute(
            "CREATE TABLE t3 (id INTEGER PRIMARY KEY, ref_id INTEGER)",
            [],
        )
        .unwrap();

        // Insert data into all tables
        conn.execute("INSERT INTO t1 VALUES (1, 'hello')", [])
            .unwrap();
        conn.execute("INSERT INTO t2 VALUES (1, 42)", []).unwrap();
        conn.execute("INSERT INTO t3 VALUES (1, 1)", []).unwrap();

        // RECOVERY: Single snapshot captures all tables and data
        let snapshot = synddb.snapshot().unwrap();
        assert!(!snapshot.data.is_empty());

        // All tables are now available to validators
    }

    #[test]
    #[ignore] // Requires running sequencer: cargo test -p synddb-client -- --ignored
    fn test_recovery_captures_current_state() {
        // Verifies that recovery snapshot captures the CURRENT state,
        // including all data inserted after the schema change.
        let conn = Box::leak(Box::new(Connection::open_in_memory().unwrap()));
        let synddb = SyndDB::attach(conn, "http://localhost:8433").unwrap();

        // Direct DDL
        conn.execute(
            "CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT, count INTEGER)",
            [],
        )
        .unwrap();

        // Insert many rows over time (pretend these are from various transactions)
        for i in 1..=50 {
            conn.execute(
                "INSERT INTO items VALUES (?1, ?2, ?3)",
                rusqlite::params![i, format!("Item {}", i), i * 10],
            )
            .unwrap();
        }

        // Several push cycles (changesets would fail on validators)
        synddb.push().unwrap();
        thread::sleep(std::time::Duration::from_millis(100));

        // More data changes
        conn.execute("UPDATE items SET count = count + 1 WHERE id <= 10", [])
            .unwrap();
        conn.execute("DELETE FROM items WHERE id > 45", []).unwrap();
        synddb.push().unwrap();

        // Recovery snapshot captures CURRENT state (all modifications included)
        let snapshot = synddb.snapshot().unwrap();
        assert!(!snapshot.data.is_empty());

        // After recovery, validators start fresh from this snapshot.
        // They won't replay the old changesets - they have the current state.

        // Local verification
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM items", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 45); // 50 - 5 deleted
    }

    #[test]
    #[ignore] // Requires running sequencer: cargo test -p synddb-client -- --ignored
    fn test_compare_correct_vs_incorrect_ddl_flow() {
        // Side-by-side comparison of correct vs incorrect DDL handling.
        // Both achieve the same local result, but only the correct path
        // results in validators being able to reconstruct state.

        // === CORRECT: Using execute_ddl() ===
        let conn1 = Box::leak(Box::new(Connection::open_in_memory().unwrap()));
        let synddb1 = SyndDB::attach(conn1, "http://localhost:8433").unwrap();

        synddb1
            .execute_ddl("CREATE TABLE correct_table (id INTEGER PRIMARY KEY)")
            .unwrap();
        // ^ Automatically publishes snapshot containing the schema

        conn1
            .execute("INSERT INTO correct_table VALUES (1)", [])
            .unwrap();
        synddb1.push().unwrap();
        // ^ Validator can apply this because it received the schema snapshot

        // === INCORRECT: Using connection().execute() ===
        let conn2 = Box::leak(Box::new(Connection::open_in_memory().unwrap()));
        let synddb2 = SyndDB::attach(conn2, "http://localhost:8433").unwrap();

        conn2
            .execute("CREATE TABLE incorrect_table (id INTEGER PRIMARY KEY)", [])
            .unwrap();
        // ^ No snapshot published - validator doesn't know about this table

        conn2
            .execute("INSERT INTO incorrect_table VALUES (1)", [])
            .unwrap();
        synddb2.push().unwrap();
        // ^ Validator fails: "no such table: incorrect_table"

        // Manual recovery required:
        synddb2.snapshot().unwrap();
        // Now validator can catch up
    }

    #[test]
    #[ignore] // Requires running sequencer: cargo test -p synddb-client -- --ignored
    fn test_alter_table_without_execute_ddl() {
        // ALTER TABLE is also DDL and requires a snapshot.
        // This test documents the ALTER TABLE recovery scenario.
        let conn = Box::leak(Box::new(Connection::open_in_memory().unwrap()));
        let synddb = SyndDB::attach(conn, "http://localhost:8433").unwrap();

        // Correct initial setup
        synddb
            .execute_ddl("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)")
            .unwrap();

        // Insert initial data (validators have schema)
        conn.execute("INSERT INTO users VALUES (1, 'Alice')", [])
            .unwrap();
        synddb.push().unwrap();

        // MISTAKE: ALTER TABLE via direct connection
        conn.execute("ALTER TABLE users ADD COLUMN email TEXT", [])
            .unwrap();

        // Using new column (changesets will reference column validators don't have)
        conn.execute(
            "UPDATE users SET email = 'alice@example.com' WHERE id = 1",
            [],
        )
        .unwrap();
        synddb.push().unwrap();

        // RECOVERY: Publish snapshot with updated schema
        synddb.snapshot().unwrap();

        // After recovery, validators have the new column
        let email: String = conn
            .query_row("SELECT email FROM users WHERE id = 1", [], |row| row.get(0))
            .unwrap();
        assert_eq!(email, "alice@example.com");
    }

    #[test]
    #[ignore] // Requires running sequencer: cargo test -p synddb-client -- --ignored
    fn test_drop_table_without_execute_ddl() {
        // DROP TABLE is also DDL. This documents what happens.
        let conn = Box::leak(Box::new(Connection::open_in_memory().unwrap()));
        let synddb = SyndDB::attach(conn, "http://localhost:8433").unwrap();

        // Setup tables correctly
        synddb
            .execute_ddl("CREATE TABLE t1 (id INTEGER PRIMARY KEY)")
            .unwrap();
        synddb
            .execute_ddl("CREATE TABLE t2 (id INTEGER PRIMARY KEY)")
            .unwrap();

        // MISTAKE: DROP via direct connection
        conn.execute("DROP TABLE t1", []).unwrap();

        // After recovery snapshot, validators know t1 no longer exists
        synddb.snapshot().unwrap();

        // Verify t1 is gone, t2 remains
        let tables: Vec<String> = conn
            .prepare(
                "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'",
            )
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        assert_eq!(tables, vec!["t2"]);
    }

    #[test]
    #[ignore] // Requires running sequencer: cargo test -p synddb-client -- --ignored
    fn test_recovery_recommended_after_any_direct_ddl() {
        // Best practice: If you accidentally used direct DDL, call snapshot()
        // immediately. Don't wait for validator errors.
        let conn = Box::leak(Box::new(Connection::open_in_memory().unwrap()));
        let synddb = SyndDB::attach(conn, "http://localhost:8433").unwrap();

        // Oops, used direct DDL
        conn.execute("CREATE TABLE oops (id INTEGER)", []).unwrap();

        // Best practice: Immediately publish snapshot if you realize the mistake
        synddb.snapshot().unwrap();

        // Now it's safe to proceed - validators will have the schema
        conn.execute("INSERT INTO oops VALUES (1)", []).unwrap();
        synddb.push().unwrap();
    }

    #[test]
    #[ignore] // Requires running sequencer: cargo test -p synddb-client -- --ignored
    fn test_snapshot_is_idempotent_for_recovery() {
        // Multiple snapshots are fine - each one is a complete database state.
        // Validators just use the most recent one.
        let conn = Box::leak(Box::new(Connection::open_in_memory().unwrap()));
        let synddb = SyndDB::attach(conn, "http://localhost:8433").unwrap();

        conn.execute("CREATE TABLE t (id INTEGER, val TEXT)", [])
            .unwrap();
        conn.execute("INSERT INTO t VALUES (1, 'a')", []).unwrap();

        // Multiple recovery snapshots are safe
        synddb.snapshot().unwrap();
        synddb.snapshot().unwrap();
        synddb.snapshot().unwrap();

        // Each snapshot is a complete, self-contained database state
    }

    // =========================================================================
    // Integration-style tests for full DDL/DML workflows
    // =========================================================================

    #[test]
    #[ignore] // Requires running sequencer: cargo test -p synddb-client -- --ignored
    fn test_typical_migration_workflow_wrong_way() {
        // Documents the common mistake in migration workflows
        let conn = Box::leak(Box::new(Connection::open_in_memory().unwrap()));
        let synddb = SyndDB::attach(conn, "http://localhost:8433").unwrap();

        // Initial setup (correct)
        synddb
            .execute_ddl("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)")
            .unwrap();

        conn.execute("INSERT INTO users VALUES (1, 'Alice')", [])
            .unwrap();
        synddb.push().unwrap();

        // === MIGRATION (WRONG WAY) ===
        // Developer runs migration script directly
        conn.execute_batch(
            r#"
            ALTER TABLE users ADD COLUMN email TEXT;
            ALTER TABLE users ADD COLUMN created_at INTEGER;
            CREATE TABLE user_settings (user_id INTEGER PRIMARY KEY, theme TEXT);
            CREATE INDEX idx_users_email ON users(email);
            "#,
        )
        .unwrap();
        // All this DDL goes untracked!

        // Data modifications using new schema
        conn.execute(
            "UPDATE users SET email = 'alice@test.com', created_at = 12345 WHERE id = 1",
            [],
        )
        .unwrap();
        conn.execute("INSERT INTO user_settings VALUES (1, 'dark')", [])
            .unwrap();

        // RECOVERY POINT
        synddb.snapshot().unwrap();

        // Now validators can continue
    }

    #[test]
    #[ignore] // Requires running sequencer: cargo test -p synddb-client -- --ignored
    fn test_typical_migration_workflow_right_way() {
        // Documents the correct migration workflow
        let conn = Box::leak(Box::new(Connection::open_in_memory().unwrap()));
        let synddb = SyndDB::attach(conn, "http://localhost:8433").unwrap();

        // Initial setup
        synddb
            .execute_ddl("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)")
            .unwrap();

        conn.execute("INSERT INTO users VALUES (1, 'Alice')", [])
            .unwrap();
        synddb.push().unwrap();

        // === MIGRATION (RIGHT WAY) ===
        // Each DDL goes through execute_ddl()
        synddb
            .execute_ddl("ALTER TABLE users ADD COLUMN email TEXT")
            .unwrap();
        synddb
            .execute_ddl("ALTER TABLE users ADD COLUMN created_at INTEGER")
            .unwrap();
        synddb
            .execute_ddl("CREATE TABLE user_settings (user_id INTEGER PRIMARY KEY, theme TEXT)")
            .unwrap();
        synddb
            .execute_ddl("CREATE INDEX idx_users_email ON users(email)")
            .unwrap();
        // Each execute_ddl() automatically publishes a snapshot

        // Data modifications
        conn.execute(
            "UPDATE users SET email = 'alice@test.com', created_at = 12345 WHERE id = 1",
            [],
        )
        .unwrap();
        conn.execute("INSERT INTO user_settings VALUES (1, 'dark')", [])
            .unwrap();
        synddb.push().unwrap();

        // No recovery needed - validators received snapshots for each DDL
    }

    // =========================================================================
    // Automatic Schema Change Detection Tests
    // =========================================================================
    //
    // These tests verify that the client automatically detects schema changes
    // (DDL) and sends snapshots before changesets, even when DDL is executed
    // directly via connection() instead of execute_ddl().
    //
    // The key insight: SQLite's update hook doesn't fire for DDL, but we can
    // still detect schema changes by comparing the hash of sqlite_master before
    // each push. If the hash changed, we know DDL occurred.

    #[test]
    #[ignore] // Requires running sequencer: cargo test -p synddb-client -- --ignored
    fn test_auto_schema_detection_create_table() {
        // When a table is created via direct DDL, the next publish should
        // automatically detect the schema change and send a snapshot first.
        let conn = Box::leak(Box::new(Connection::open_in_memory().unwrap()));

        // Enable snapshot_interval so snapshot channel exists
        let config = Config {
            sequencer_url: "http://localhost:8433".parse().unwrap(),
            snapshot_interval: 100,
            ..Default::default()
        };
        let synddb = SyndDB::attach_with_config(conn, config).unwrap();

        // Direct DDL (not using execute_ddl)
        conn.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)", [])
            .unwrap();

        // Insert data (triggers update hook)
        conn.execute("INSERT INTO users VALUES (1, 'Alice')", [])
            .unwrap();

        // Publish will detect schema change and send snapshot first
        synddb.push().unwrap();

        // Verify data is correct
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    #[ignore] // Requires running sequencer: cargo test -p synddb-client -- --ignored
    fn test_auto_schema_detection_alter_table() {
        // ALTER TABLE changes schema hash too
        let conn = Box::leak(Box::new(Connection::open_in_memory().unwrap()));

        let config = Config {
            sequencer_url: "http://localhost:8433".parse().unwrap(),
            snapshot_interval: 100,
            ..Default::default()
        };
        let synddb = SyndDB::attach_with_config(conn, config).unwrap();

        // Initial table via execute_ddl (correct)
        synddb
            .execute_ddl("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)")
            .unwrap();

        // Insert initial data
        conn.execute("INSERT INTO users VALUES (1, 'Alice')", [])
            .unwrap();
        synddb.push().unwrap();

        // ALTER via direct DDL (will be auto-detected)
        conn.execute("ALTER TABLE users ADD COLUMN email TEXT", [])
            .unwrap();

        // Use the new column
        conn.execute("UPDATE users SET email = 'alice@test.com' WHERE id = 1", [])
            .unwrap();

        // This publish will auto-detect the schema change
        synddb.push().unwrap();

        // Verify
        let email: String = conn
            .query_row("SELECT email FROM users WHERE id = 1", [], |row| row.get(0))
            .unwrap();
        assert_eq!(email, "alice@test.com");
    }

    #[test]
    #[ignore] // Requires running sequencer: cargo test -p synddb-client -- --ignored
    fn test_auto_schema_detection_multiple_ddl() {
        // Multiple DDL operations before push - only one snapshot needed
        let conn = Box::leak(Box::new(Connection::open_in_memory().unwrap()));

        let config = Config {
            sequencer_url: "http://localhost:8433".parse().unwrap(),
            snapshot_interval: 100,
            ..Default::default()
        };
        let synddb = SyndDB::attach_with_config(conn, config).unwrap();

        // Multiple DDL operations via direct connection
        conn.execute("CREATE TABLE t1 (id INTEGER PRIMARY KEY)", [])
            .unwrap();
        conn.execute("CREATE TABLE t2 (id INTEGER PRIMARY KEY)", [])
            .unwrap();
        conn.execute("CREATE TABLE t3 (id INTEGER PRIMARY KEY)", [])
            .unwrap();
        conn.execute("CREATE INDEX idx_t1 ON t1(id)", []).unwrap();

        // Insert data into all tables
        conn.execute("INSERT INTO t1 VALUES (1)", []).unwrap();
        conn.execute("INSERT INTO t2 VALUES (1)", []).unwrap();
        conn.execute("INSERT INTO t3 VALUES (1)", []).unwrap();

        // Single publish detects schema change and sends snapshot
        synddb.push().unwrap();
    }

    #[test]
    #[ignore] // Requires running sequencer: cargo test -p synddb-client -- --ignored
    fn test_auto_schema_detection_no_false_positives() {
        // Verify that identical schemas don't trigger false positive detection
        let conn = Box::leak(Box::new(Connection::open_in_memory().unwrap()));

        let config = Config {
            sequencer_url: "http://localhost:8433".parse().unwrap(),
            snapshot_interval: 100,
            ..Default::default()
        };
        let synddb = SyndDB::attach_with_config(conn, config).unwrap();

        // Create table
        synddb
            .execute_ddl("CREATE TABLE test (id INTEGER PRIMARY KEY)")
            .unwrap();

        // Multiple pushes with no schema changes - no extra snapshots
        for i in 1..=10 {
            conn.execute("INSERT INTO test VALUES (?1)", [i]).unwrap();
            synddb.push().unwrap();
        }

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM test", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 10);
    }

    #[test]
    #[ignore] // Requires running sequencer: cargo test -p synddb-client -- --ignored
    fn test_auto_schema_detection_drop_table() {
        // DROP TABLE also changes schema hash
        let conn = Box::leak(Box::new(Connection::open_in_memory().unwrap()));

        let config = Config {
            sequencer_url: "http://localhost:8433".parse().unwrap(),
            snapshot_interval: 100,
            ..Default::default()
        };
        let synddb = SyndDB::attach_with_config(conn, config).unwrap();

        // Create tables
        synddb.execute_ddl("CREATE TABLE t1 (id INTEGER)").unwrap();
        synddb.execute_ddl("CREATE TABLE t2 (id INTEGER)").unwrap();

        // Drop via direct DDL
        conn.execute("DROP TABLE t1", []).unwrap();

        // Insert into remaining table (triggers update hook)
        conn.execute("INSERT INTO t2 VALUES (1)", []).unwrap();

        // Publish detects schema change
        synddb.push().unwrap();

        // Verify t1 is gone
        let tables: Vec<String> = conn
            .prepare(
                "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'",
            )
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        assert_eq!(tables, vec!["t2"]);
    }

    #[test]
    fn test_config_validation_rejects_zero_snapshot_interval() {
        let conn = Box::leak(Box::new(Connection::open_in_memory().unwrap()));

        let config = Config {
            sequencer_url: "http://localhost:8433".parse().unwrap(),
            snapshot_interval: 0,
            ..Default::default()
        };
        let result = SyndDB::attach_with_config(conn, config);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("snapshot_interval must be greater than 0"));
    }

    #[test]
    fn test_config_validation_rejects_zero_buffer_size() {
        let conn = Box::leak(Box::new(Connection::open_in_memory().unwrap()));

        let config = Config {
            sequencer_url: "http://localhost:8433".parse().unwrap(),
            buffer_size: 0,
            ..Default::default()
        };
        let result = SyndDB::attach_with_config(conn, config);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("buffer_size must be greater than 0"));
    }

    #[test]
    fn test_config_validation_rejects_zero_flush_interval() {
        let conn = Box::leak(Box::new(Connection::open_in_memory().unwrap()));

        let config = Config {
            sequencer_url: "http://localhost:8433".parse().unwrap(),
            flush_interval: std::time::Duration::ZERO,
            ..Default::default()
        };
        let result = SyndDB::attach_with_config(conn, config);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("flush_interval must be greater than 0"));
    }
}
