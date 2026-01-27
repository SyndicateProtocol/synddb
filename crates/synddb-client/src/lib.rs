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
//! // Use the connection for normal `SQLite` operations
//! synddb.connection().execute(
//!     "CREATE TABLE IF NOT EXISTS trades (id INTEGER, amount INTEGER)",
//!     [],
//! )?;
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
//! println!("Published: {} changesets", stats.published_changesets);
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
//! synddb.publish()?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! # Publishing Strategies
//!
//! `SyndDB` supports different strategies for when to publish changesets:
//!
//! - **Timer** (default): Automatically publishes every 1 second
//! - **Manual**: Only publishes when you call [`SyndDB::publish()`]
//!
//! Configure via [`Config::publish_strategy`]:
//!
//! ```rust,no_run
//! use synddb_client::{Config, PublishStrategy, SyndDB};
//!
//! let config = Config {
//!     sequencer_url: "http://sequencer:8433".parse().unwrap(),
//!     publish_strategy: PublishStrategy::Manual,
//!     ..Default::default()
//! };
//! let synddb = SyndDB::open_with_config("app.db", config)?;
//!
//! // With Manual strategy, you control when to publish
//! synddb.connection().execute("INSERT INTO trades VALUES (1, 100)", [])?;
//! synddb.publish()?; // Explicitly send to sequencer
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
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
//! println!("Published: {}", stats.published_changesets);
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
pub use config::{Config, PublishStrategy};
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
    /// Whether to auto-snapshot after DDL statements
    auto_snapshot_after_ddl: bool,
    /// Session monitor for capturing changesets
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
    /// use synddb_client::{Config, PublishStrategy, SyndDB};
    ///
    /// let config = Config {
    ///     sequencer_url: "http://sequencer:8433".parse().unwrap(),
    ///     publish_strategy: PublishStrategy::Manual,
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
        info!("Publish strategy: {:?}", config.publish_strategy);

        // Create shared stats handle
        let stats = stats::new_stats_handle();

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

        let synddb = Self {
            conn,
            sequencer_url: config.sequencer_url.clone(),
            auto_snapshot_after_ddl: config.auto_snapshot_after_ddl,
            monitor: Some(monitor),
            changeset_shutdown_tx,
            snapshot_shutdown_tx,
            changeset_handle: Some(changeset_handle),
            snapshot_handle,
            recovery,
            chain_monitor,
            stats,
        };

        // Auto-snapshot on attach if enabled and database has existing tables
        if config.auto_snapshot_on_attach && Self::has_existing_tables(conn) {
            info!("Database has existing tables, creating initial snapshot for validator bootstrapping");
            if let Err(e) = synddb.publish_snapshot() {
                warn!(
                    "Failed to create initial snapshot on attach: {}. Continuing without it.",
                    e
                );
            }
        }

        info!("SyndDB client attached successfully");

        Ok(synddb)
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
    /// 4. Publishes changesets after commit
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
        self.publish()?;
        Ok(result)
    }

    // =========================================================================
    // Publishing
    // =========================================================================

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

    /// Create a local snapshot of the database (does NOT send to sequencer)
    ///
    /// This captures the full current state of the database as a portable `SQLite` file.
    /// The snapshot is returned but **not sent to the sequencer**. Use this when you
    /// need the snapshot data locally (e.g., for backup, testing, or manual transfer).
    ///
    /// # Important
    ///
    /// This method only creates a local snapshot. To create AND publish a snapshot
    /// to the sequencer (the typical use case), use [`publish_snapshot()`] instead.
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
    /// [`publish_snapshot()`]: Self::publish_snapshot
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
    /// This is consistent with [`publish()`] for changesets - both methods send data
    /// to the sequencer immediately.
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
    /// let snapshot = synddb.publish_snapshot()?;
    /// println!("Published snapshot: {} bytes at sequence {}", snapshot.data.len(), snapshot.sequence);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// [`create_snapshot()`]: Self::create_snapshot
    /// [`publish()`]: Self::publish
    pub fn publish_snapshot(&self) -> Result<Snapshot> {
        let snapshot = self.create_snapshot()?;

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
    /// and automatically publishes a snapshot afterward if `auto_snapshot_after_ddl`
    /// is enabled in the configuration.
    ///
    /// Use this for schema changes instead of `connection().execute_batch()` to ensure
    /// validators can reconstruct the schema.
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
        self.conn.execute_batch(sql)?;

        if self.auto_snapshot_after_ddl && Self::is_ddl(sql) {
            info!("DDL executed, creating automatic snapshot");
            self.publish_snapshot()?;
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
    /// Returns information about pending changesets, published count, and health status.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use synddb_client::SyndDB;
    /// # let synddb = SyndDB::open("app.db", "http://sequencer:8433")?;
    /// let stats = synddb.stats();
    /// println!("Pending: {} changesets", stats.pending_changesets);
    /// println!("Published: {} changesets", stats.published_changesets);
    /// println!("Failed: {} attempts", stats.failed_publishes);
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

    /// Get the number of changesets waiting to be published
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

    /// Get the total number of successfully published changesets
    pub fn published_count(&self) -> u64 {
        self.stats.published_count()
    }

    // =========================================================================
    // Lifecycle
    // =========================================================================

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

        // Attach SyndDB with auto_snapshot_on_attach enabled
        // This should attempt to publish a snapshot (will fail since no sequencer, but shouldn't panic)
        let config = Config {
            sequencer_url: "http://localhost:8433".parse().unwrap(),
            auto_snapshot_on_attach: true,
            ..Default::default()
        };

        let _synddb = SyndDB::attach_with_config(conn, config).unwrap();
    }

    #[test]
    fn test_attach_with_disabled_auto_snapshot() {
        // Create a database with existing tables
        let conn = Box::leak(Box::new(Connection::open_in_memory().unwrap()));
        conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY)", [])
            .unwrap();

        // Attach SyndDB with auto_snapshot_on_attach disabled
        let config = Config {
            sequencer_url: "http://localhost:8433".parse().unwrap(),
            auto_snapshot_on_attach: false,
            auto_snapshot_after_ddl: false,
            ..Default::default()
        };

        let _synddb = SyndDB::attach_with_config(conn, config).unwrap();
    }
}
