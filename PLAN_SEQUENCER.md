# PLAN_SEQUENCER.md - Lightweight SQLite Monitor and Publisher

## Overview

The synddb-sequencer is a zero-configuration Rust process that attaches to any SQLite database using the **SQLite Session Extension** to capture deterministic changesets. It publishes logical database changes (INSERT/UPDATE/DELETE operations) to multiple DA layers. This approach is far more robust than WAL parsing and ensures validators can deterministically re-derive state. It requires zero application code changes and works with SQLite databases from any programming language.

**Note:** The sequencer runs as a **sidecar process** - a separate process that runs alongside the application. While we call it the "sequencer" (reflecting its role in ordering and publishing transactions), it's architecturally deployed as a sidecar.

## Architecture

```
┌──────────────────────────────────────────────────────────────┐
│                  Application (Any Language)                  │
│         Python/JS/Go/Rust/Java → SQLite API → app.db        │
│                              ↑                               │
│                    HTTP localhost:8432                       │
│                    (Deposit Events)                          │
└──────────────────────────────────────────────────────────────┘
                                │
                    Database Transactions (via Session Extension)
                                ↓
┌──────────────────────────────────────────────────────────────┐
│                      synddb-sequencer                          │
│ ┌─────────────────┐  ┌─────────────┐  ┌─────────────┐      │
│ │ Session Monitor │→ │   Batcher   │→ │  Attestor   │      │
│ │ (Changesets)    │  │ (+ Snapshot)│  │(Sign+Compress)     │
│ └─────────────────┘  └─────────────┘  └─────────────┘      │
│        ↓                                        ↓             │
│  Logical Changes                      Signed Changesets     │
│  (INSERT/UPDATE/DELETE)               + Periodic Snapshots  │
│        ↓                                        ↓             │
│ ┌──────────────────────────────────────────────────┐        │
│ │            Multi-DA Publisher                     │        │
│ │  ┌─────────┐ ┌─────────┐ ┌──────┐ ┌─────────┐  │        │
│ │  │Celestia │ │EigenDA  │ │ IPFS │ │ Arweave │  │        │
│ │  └─────────┘ └─────────┘ └──────┘ └─────────┘  │        │
│ └──────────────────────────────────────────────────┘        │
│                                                              │
│ ┌──────────────────────────────────────────────────┐        │
│ │            Deposit Monitor                        │        │
│ │  ┌────────────────┐    ┌────────────────────┐   │        │
│ │  │ Chain Monitor  │───▶│  Deposit HTTP API  │   │        │
│ │  │ (Bridge Events)│    │  localhost:8432    │   │        │
│ │  └────────────────┘    └────────────────────┘   │        │
│ └──────────────────────────────────────────────────┘        │
└──────────────────────────────────────────────────────────────┘
                                ↑
                          Event Monitoring
                                │
┌──────────────────────────────────────────────────────────────┐
│                    Blockchain (L1/L2)                        │
│                 Bridge Contract (Deposits)                   │
└──────────────────────────────────────────────────────────────┘

Key Benefits:
- Session Extension: Official SQLite API (no brittle WAL parsing)
- Deterministic: Logical changes (not physical pages)
- Compact: Only changed rows (not full database pages)
- Safe: Periodic snapshots provide recovery points
```

## Core Libraries

```toml
[dependencies]
# Core SQLite monitoring
rusqlite = { version = "0.32", features = ["bundled", "backup", "session"] }
# Session extension provides deterministic changesets (official SQLite API)

# Compression and serialization
zstd = "0.13"  # Fast compression with good ratios
bincode = "1.3"  # Efficient binary serialization
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"

# DA Layer clients
celestia-client = "0.2"  # Celestia DA
eigenda-rust = "0.1"  # EigenDA client
ipfs-api = "0.11"  # IPFS client
arweave-rs = "0.1"  # Arweave client

# Async runtime and networking
tokio = { version = "1.35", features = ["full"] }
hyper = { version = "1.0", features = ["full"] }
tower = "0.4"  # Middleware stack for retries/timeouts
axum = { version = "0.7", features = ["sse"] }  # HTTP server with SSE support

# Blockchain monitoring for deposits
ethers = "2.0"  # Ethereum client for monitoring bridge events

# GCP Confidential Space and Ethereum signing
gcp-auth = "0.10"  # GCP authentication
google-cloud-secretmanager = "0.6"  # Secret Manager for key storage
google-cloud-default = "0.6"  # Workload Identity support
k256 = { version = "0.13", features = ["ecdsa", "sha256"] }  # secp256k1 for Ethereum
sha3 = "0.10"  # Keccak256 for Ethereum address derivation
reqwest = { version = "0.11", features = ["json"] }  # Attestation token fetching
hex = "0.4"  # Hex encoding for addresses

# Configuration and monitoring
config = "0.14"  # YAML/TOML configuration
google-cloud-logging = "0.6"  # GCP Cloud Logging integration
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }
sysinfo = "0.30"  # System monitoring

# Error handling and utilities
anyhow = "1.0"
thiserror = "1.0"
backoff = "0.4"  # Exponential backoff for retries
parking_lot = "0.12"  # Better synchronization primitives
crossbeam-channel = "0.5"  # Multi-producer multi-consumer channels
```

## Directory Structure

```
synddb-sequencer/
├── Cargo.toml
├── src/
│   ├── main.rs                    # Entry point, CLI args
│   ├── lib.rs                     # Public API
│   ├── config.rs                  # Configuration structures
│   ├── monitor/
│   │   ├── mod.rs                 # Session monitoring module
│   │   ├── session_tracker.rs     # SQLite Session Extension wrapper
│   │   ├── changeset_reader.rs    # Parse changeset format
│   │   └── hooks.rs               # Commit hooks for changeset extraction
│   ├── batch/
│   │   ├── mod.rs                 # Batching logic
│   │   ├── accumulator.rs         # Accumulate operations
│   │   ├── timer.rs               # Time/size based triggers
│   │   └── snapshot.rs            # Full database snapshot creation
│   ├── attestor/
│   │   ├── mod.rs                 # Attestation and signing
│   │   ├── key_manager.rs         # Ethereum key management in TEE
│   │   ├── signer.rs              # Sign batches with secp256k1
│   │   └── compressor.rs          # Zstd compression (compress-then-sign)
│   ├── publish/
│   │   ├── mod.rs                 # Publishing orchestration
│   │   ├── celestia.rs            # Celestia publisher
│   │   ├── eigenda.rs             # EigenDA publisher
│   │   ├── ipfs.rs                # IPFS publisher
│   │   ├── arweave.rs             # Arweave publisher
│   │   ├── retry.rs               # Retry logic
│   │   └── manifest.rs            # Track published batches (sequence, DA location, hash)
│   ├── deposits/
│   │   ├── mod.rs                 # Deposit handling module
│   │   ├── chain_monitor.rs       # Blockchain event monitoring
│   │   ├── queue.rs               # Deposit queue management
│   │   └── api.rs                 # HTTP API for applications
│   ├── tee/
│   │   ├── mod.rs                 # GCP Confidential Space TEE
│   │   └── attestation.rs         # Fetch attestation tokens from metadata service
│   └── utils/
│       ├── mod.rs
│       ├── checksum.rs            # Data integrity
│       └── sqlite_utils.rs        # SQLite helpers
├── config/
│   ├── default.yaml               # Default configuration
│   └── example.yaml               # Example with all options
├── tests/
│   ├── integration/               # Integration tests
│   └── benchmarks/                # Performance benchmarks
└── README.md
```

## Core Components

### 1. Session Monitor (Data Changes + Schema Tracking)

The Session Monitor uses SQLite's official Session Extension to capture deterministic changesets for data changes. It also tracks schema changes (DDL statements) separately to ensure validators can replicate the complete database state including schema evolution.

```rust
// src/monitor/session_tracker.rs
use rusqlite::{Connection, hooks::Action};

pub struct SessionMonitor {
    db_path: PathBuf,
    conn: Connection,
    session: rusqlite::Session<'static>,
    changeset_tx: Sender<Changeset>,
    schema_tx: Sender<SchemaChange>,
    last_schema_version: i32,
}

impl SessionMonitor {
    pub async fn new(
        db_path: PathBuf,
        changeset_tx: Sender<Changeset>,
        schema_tx: Sender<SchemaChange>
    ) -> Result<Self> {
        // Open database connection
        let mut conn = Connection::open(&db_path)?;

        // Get current schema version
        let last_schema_version: i32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;

        // Create session to track data changes
        // Session extension must be enabled at compile time:
        // RUSTFLAGS="-C link-arg=-lsqlite3" cargo build --features session
        let session = rusqlite::Session::new(&conn)?;

        // Attach to all tables (None = all tables)
        session.attach(None)?;

        // Install update hook to detect schema changes
        // This fires BEFORE commit hook, allowing us to capture DDL
        conn.update_hook(Some(Self::on_update));

        // Install commit hook to capture changesets after each transaction
        conn.commit_hook(Some(Self::on_commit));

        Ok(Self {
            db_path,
            conn,
            session,
            changeset_tx,
            schema_tx,
            last_schema_version,
        })
    }

    /// Update hook called for every INSERT/UPDATE/DELETE
    /// We use this to detect writes to sqlite_schema (DDL changes)
    fn on_update(&mut self, action: Action, db: &str, table: &str, rowid: i64) {
        if table == "sqlite_schema" || table == "sqlite_master" {
            // Schema change detected! Capture it
            match self.capture_schema_change() {
                Ok(schema_change) => {
                    if let Err(e) = self.schema_tx.blocking_send(schema_change) {
                        error!("Failed to send schema change: {}", e);
                    }
                }
                Err(e) => {
                    error!("Failed to capture schema change: {}", e);
                }
            }
        }
    }

    fn capture_schema_change(&mut self) -> Result<SchemaChange> {
        // Get new schema version (applications should increment this on ALTER)
        let new_version: i32 = self.conn.pragma_query_value(None, "user_version", |row| row.get(0))?;

        // Get full schema DDL
        let schema_sql: Vec<String> = self.conn
            .prepare("SELECT sql FROM sqlite_schema WHERE sql IS NOT NULL ORDER BY type, name")?
            .query_map([], |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()?;

        // Get schema hash for quick comparison
        let schema_hash = blake3::hash(&schema_sql.join("\n").as_bytes());

        Ok(SchemaChange {
            old_version: self.last_schema_version,
            new_version,
            ddl_statements: schema_sql,
            schema_hash: schema_hash.to_hex().to_string(),
            timestamp: SystemTime::now(),
        })
    }

    /// Commit hook called after each successful transaction
    fn on_commit(&mut self) -> bool {
        match self.extract_changeset() {
            Ok(changeset) => {
                // Send to batcher
                if let Err(e) = self.tx.blocking_send(changeset) {
                    error!("Failed to send changeset: {}", e);
                }
                false // Allow commit to proceed
            }
            Err(e) => {
                error!("Failed to extract changeset: {}", e);
                false // Allow commit to proceed (logging only)
            }
        }
    }

    fn extract_changeset(&mut self) -> Result<Changeset> {
        // Extract changeset from session
        // This contains all INSERT/UPDATE/DELETE operations since last extract
        let changeset_blob = self.session.changeset()?;

        // Parse changeset to understand what changed (for logging/metrics)
        let metadata = self.parse_changeset_metadata(&changeset_blob)?;

        Ok(Changeset {
            data: changeset_blob,
            table_changes: metadata.table_changes,
            operation_count: metadata.operation_count,
            timestamp: SystemTime::now(),
        })
    }

    fn parse_changeset_metadata(&self, changeset: &[u8]) -> Result<ChangesetMetadata> {
        // Parse changeset format to extract metadata
        // Format: https://www.sqlite.org/sessionintro.html
        let mut operations = Vec::new();
        let mut offset = 0;

        while offset < changeset.len() {
            let table_name = self.read_table_name(changeset, &mut offset)?;
            let op_type = self.read_operation_type(changeset, &mut offset)?;
            let row_count = self.read_row_count(changeset, &mut offset)?;

            operations.push(TableChange {
                table: table_name,
                operation: op_type,
                rows: row_count,
            });
        }

        Ok(ChangesetMetadata {
            table_changes: operations,
            operation_count: operations.iter().map(|op| op.rows).sum(),
        })
    }

    pub async fn run(mut self) -> Result<()> {
        // Session monitor is passive - changesets are extracted via commit hook
        // This task just keeps the connection alive and handles shutdown
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;

            // Periodic health check
            self.conn.execute("SELECT 1", [])?;
        }
    }
}

#[derive(Debug, Clone)]
pub struct Changeset {
    /// Binary changeset blob (SQLite Session format)
    pub data: Vec<u8>,
    /// Metadata about what tables changed
    pub table_changes: Vec<TableChange>,
    /// Total number of row operations
    pub operation_count: usize,
    pub timestamp: SystemTime,
}

#[derive(Debug, Clone)]
pub struct TableChange {
    pub table: String,
    pub operation: OperationType,
    pub rows: usize,
}

#[derive(Debug, Clone, Copy)]
pub enum OperationType {
    Insert,
    Update,
    Delete,
}

#[derive(Debug, Clone)]
pub struct SchemaChange {
    /// Previous schema version (from user_version pragma)
    pub old_version: i32,
    /// New schema version (applications should increment on ALTER)
    pub new_version: i32,
    /// Full DDL statements for all tables, indexes, triggers, views
    pub ddl_statements: Vec<String>,
    /// Hash of schema for quick comparison
    pub schema_hash: String,
    pub timestamp: SystemTime,
}
```

**Important: Application Schema Change Pattern**
```sql
-- Applications MUST increment user_version when changing schema
BEGIN TRANSACTION;
  ALTER TABLE users ADD COLUMN age INTEGER;
  PRAGMA user_version = 2;  -- Increment version
COMMIT;

-- This allows sequencer to detect and publish schema changes
```

### Why Session Extension (Changesets) is the Right Choice

**Changesets vs. Patchsets:**
- **Changesets** include original values for UPDATE/DELETE operations, making them **easier to audit** on validators
- Validators can verify: "Was the old value really X before updating to Y?"
- Patchsets only store new values, requiring trust that the primary had correct old values
- Changesets support full reversibility (rollback) and better conflict detection

**Changesets vs. WAL Parsing:**
1. **Official SQLite API** - Maintained by SQLite team, stable across versions (WAL format is internal/undocumented)
2. **Not Brittle** - SQLite handles format internally, we just consume changesets (WAL parsing breaks on format changes)
3. **Deterministic** - Captures logical changes (INSERT/UPDATE/DELETE), not physical page changes
4. **Compact** - Only stores changed rows with primary keys, not entire database pages (10-100x smaller)
5. **Validator-Friendly** - Replicas apply exact same logical changes deterministically via `sqlite3changeset_apply()`
6. **Conflict Detection** - Built-in conflict resolution when applying to replicas (WAL has none)
7. **Auditable** - Changesets can be inspected to see what changed (WAL pages are opaque binary data)

**Example: How Validators Audit Changesets**
```rust
// Validator receives changeset and can inspect it before applying
for change in changeset.iter() {
    match change.operation {
        Update { table, pk, old_values, new_values } => {
            // Verify: "Did users.id=1 really have name='Alice' before changing to 'Bob'?"
            assert_eq!(old_values["name"], "Alice"); // Audit check
            // Apply change knowing exact before/after state
        }
        Delete { table, pk, old_values } => {
            // Verify: "Did the row we're deleting actually exist with these values?"
            // With patchsets, we'd have no way to verify this
        }
        _ => {}
    }
}
```

### Schema Change Tracking: Snapshot-on-Schema-Change

**The Challenge:** SQLite Session Extension changesets only capture data changes (INSERT/UPDATE/DELETE), not schema changes (CREATE TABLE, ALTER TABLE, DROP INDEX, etc.). Validators need to know about schema changes to correctly apply data changesets.

**Our Solution: Automatic Snapshot on Schema Change**

When the application changes the database schema, the sequencer immediately creates and publishes a full database snapshot. This is simpler and more reliable than DDL replay, and leverages our existing snapshot infrastructure.

**Why Snapshot Instead of DDL Replay?**
- **Schema changes are rare** - Acceptable overhead (happens days/weeks apart, not continuously)
- **Simpler validators** - No DDL replay logic, no non-deterministic ALTER TABLE edge cases
- **Natural epochs** - Schema changes create clear boundaries in database history
- **Guaranteed consistency** - Snapshot contains exact schema + data state
- **Audit trail preserved** - DDL still published alongside snapshot for transparency

**How It Works:**

1. **Application Changes Schema**
   ```sql
   BEGIN TRANSACTION;
     ALTER TABLE users ADD COLUMN email TEXT;
     PRAGMA user_version = 2;  -- MUST increment version
   COMMIT;
   ```

2. **Sequencer Detects Schema Change**
   - Update hook fires when `sqlite_schema` table is modified
   - Captures full DDL via `SELECT sql FROM sqlite_schema` (for audit trail)
   - Reads new `user_version` to track migration number
   - **Immediately creates full database snapshot** (includes schema + all data)
   - Publishes `SnapshotWithSchemaChange` message

3. **Validators Receive Snapshot**
   ```rust
   // Validator processing sequence from DA layer
   match message.payload_type {
       PayloadType::SnapshotWithSchemaChange { snapshot, schema_change } => {
           // Log schema change for audit trail
           info!("Schema migration v{} -> v{}: {}",
               schema_change.old_version,
               schema_change.new_version,
               schema_change.ddl_statements.join("; ")
           );

           // Replace entire database with snapshot (includes new schema)
           validator.replace_database(snapshot)?;

           // Reset changeset tracking (new epoch begins)
           validator.reset_changeset_sequence();
       }
       PayloadType::ChangesetBatch => {
           // Schema guaranteed to match (snapshot resets state)
           validator.apply_changesets(changesets)?;
       }
       PayloadType::Snapshot => {
           // Regular periodic snapshot (no schema change)
           validator.replace_database(snapshot)?;
       }
   }
   ```

**Benefits:**
- **Simplicity** - Validators don't need DDL replay logic
- **Guaranteed Consistency** - Snapshot has exact schema + data state
- **Natural Epochs** - Schema changes create clear boundaries in history
- **Fast Sync** - New validators can start from latest schema snapshot
- **Audit Trail** - DDL statements still published for transparency
- **No Edge Cases** - Avoids ALTER TABLE determinism issues

**Message Size Impact:**
Schema changes are rare (typically days/weeks apart), so the snapshot overhead is acceptable. A 1GB database snapshot on schema change, happening weekly, is far better than complex DDL replay logic that might break.

**Application Requirements:**
- Applications SHOULD use `PRAGMA user_version` to track schema versions
- Increment `user_version` in same transaction as schema changes (helps with debugging/audit)
- No other requirements - snapshot captures everything

## SQLite Replication Edge Cases

### Edge Cases Handled Automatically ✅

**1. Non-Deterministic Functions (random(), datetime('now'), etc.)**
- **Problem:** `INSERT INTO events VALUES (random(), datetime('now'))` would produce different values on validators
- **Solution:** Session Extension captures **actual written values**, not SQL functions
- **Changeset contains:** `INSERT INTO events VALUES (123456, '2024-01-15 10:30:00')`
- **Note:** `SQLITE_DETERMINISTIC` flag is for user-defined functions only and doesn't affect built-in functions
- **Status:** ✅ Automatically handled by changesets (no configuration needed)

**2. AUTOINCREMENT and ROWID**
- **Problem:** Primary and validators might have different `sqlite_sequence` state
- **Solution:** Changesets include explicit rowid values for all operations
- **Changeset contains:** `INSERT INTO users (rowid=5, name='Alice')`
- **Status:** ✅ Automatically handled by changesets

**3. Transaction Rollbacks**
- **Problem:** Failed transactions shouldn't be replicated
- **Solution:** Session Extension only captures committed transactions
- **Commit hook fires after COMMIT succeeds**
- **Status:** ✅ Automatically handled by commit hook timing

**4. Write Concurrency**
- **Problem:** N/A - SQLite has single-writer model
- **Status:** ✅ Not applicable

### Edge Cases Requiring Configuration ⚠️

**5. PRAGMA Settings**
- **Problem:** Mismatched PRAGMA settings between primary and validators
- **Solution:** Validators MUST use same settings as primary
- **Critical PRAGMAs:**
  ```sql
  PRAGMA foreign_keys = ON;        -- Must match primary
  PRAGMA recursive_triggers = OFF; -- Must match primary
  PRAGMA secure_delete = OFF;      -- Must match primary
  ```
- **Implementation:** Include PRAGMA settings in snapshots
- **Status:** ⚠️ TODO - Add PRAGMA capture to snapshots

**6. Triggers**
- **Problem:** Triggers fire on primary, generating additional writes captured in changesets
- **Example:**
  ```sql
  -- Primary has trigger:
  CREATE TRIGGER log_updates AFTER UPDATE ON users
  BEGIN
    INSERT INTO audit_log VALUES (NEW.id, datetime('now'));
  END;

  -- Application: UPDATE users SET name='Bob' WHERE id=1
  -- Changeset captures BOTH:
  --   1. UPDATE users SET name='Bob' WHERE id=1
  --   2. INSERT INTO audit_log VALUES (1, '2024-01-15 10:30:00')
  ```
- **Solution:** Triggers captured in schema (DDL), effects captured in changesets
- **Validator behavior:** Applies changeset WITHOUT re-firing triggers (they already fired on primary)
- **Status:** ✅ Session Extension includes trigger-generated changes in changesets

**7. Virtual Tables (FTS5, R*Tree, etc.)**
- **Problem:** Virtual tables may not support Session Extension
- **Solution:** Test each virtual table type for compatibility
- **Known issues:**
  - FTS5 (Full-Text Search) - May need special handling
  - R*Tree (Spatial index) - May need special handling
- **Status:** ⚠️ TODO - Test virtual table support

**8. Database Encoding (UTF-8 vs UTF-16)**
- **Problem:** Primary and validators must use same text encoding
- **Solution:** Enforce `PRAGMA encoding = 'UTF-8'` everywhere
- **Status:** ⚠️ TODO - Add encoding check to snapshot validation

**9. User-Defined Functions (UDFs)**
- **Problem:** Applications may register custom functions that don't exist on validators
- **Examples:**
  ```python
  # Python app registers custom function
  conn.create_function("my_hash", 1, my_custom_hash)

  # Then uses it
  conn.execute("INSERT INTO data VALUES (my_hash('input'))")
  ```
- **Solution:** Values captured in changesets (function already executed on primary)
- **Validator requirement:** Validators don't need the UDF definition
- **Note:** `SQLITE_DETERMINISTIC` flag only matters if UDF is used in indexes/constraints
- **Status:** ✅ Automatically handled (values captured, not function calls)

### Application Requirements for Determinism

**Applications MUST:**
1. Use `PRAGMA journal_mode = WAL` (required for Session Extension)
2. NOT rely on `PRAGMA user_version` for application logic (only use for schema tracking)
3. Avoid `PRAGMA foreign_keys` changes mid-operation

**Applications SHOULD:**
1. Use explicit transactions for multi-statement operations
2. Use AUTOINCREMENT for tables that need stable IDs across replicas
3. Avoid time-dependent DEFAULT values like `DEFAULT (datetime('now'))`

**Applications CAN safely use:**
1. Non-deterministic functions (values captured in changesets) ✅
2. Triggers (effects captured in changesets) ✅
3. Foreign keys (constraints checked on primary) ✅
4. CHECK constraints (validated on primary) ✅

### 2. Batcher

Accumulates changesets and triggers publishing based on time/size thresholds. Also handles periodic snapshot creation.

```rust
// src/batch/accumulator.rs
pub struct Batcher {
    db_path: PathBuf,
    changesets: Vec<Changeset>,
    buffer_size: usize,
    last_flush: Instant,
    last_snapshot: Instant,
    batch_count: usize,
    sequence: AtomicU64,
    config: BatchConfig,
}

pub struct BatchConfig {
    max_batch_size: usize,      // 1MB default
    max_batch_age: Duration,    // 1 second default
    snapshot_interval: Duration, // 60 mins default
    snapshot_threshold: usize,   // 1000 batches default
}

#[derive(Debug, Clone)]
pub enum BatchPayload {
    /// Full database snapshot triggered by schema change
    /// Includes DDL statements for audit trail
    SnapshotWithSchemaChange {
        snapshot_data: Vec<u8>,
        schema_change: SchemaChange,
        sequence: u64,
        timestamp: SystemTime,
    },
    /// Incremental changesets (deterministic replay)
    ChangesetBatch {
        changesets: Vec<Changeset>,
        sequence: u64,
        timestamp: SystemTime,
    },
    /// Full database snapshot (periodic recovery point)
    Snapshot {
        data: Vec<u8>,
        sequence: u64,
        timestamp: SystemTime,
    },
}

impl Batcher {
    pub async fn run(
        mut self,
        changeset_rx: Receiver<Changeset>,
        schema_rx: Receiver<SchemaChange>,
        tx: Sender<BatchPayload>
    ) {
        let mut flush_timer = tokio::time::interval(self.config.max_batch_age);

        loop {
            tokio::select! {
                Some(schema_change) = schema_rx.recv() => {
                    // Schema changes trigger immediate snapshot
                    // Discard pending changesets (snapshot includes all data)
                    self.changesets.clear();
                    self.buffer_size = 0;

                    // Create snapshot with current schema + data
                    let snapshot_data = self.create_snapshot().await?;

                    // Publish snapshot with schema change metadata
                    tx.send(BatchPayload::SnapshotWithSchemaChange {
                        snapshot_data,
                        schema_change: schema_change.clone(),
                        sequence: self.next_sequence(),
                        timestamp: SystemTime::now(),
                    }).await?;

                    warn!(
                        "SCHEMA CHANGE v{} -> v{}: Published snapshot ({} bytes)",
                        schema_change.old_version,
                        schema_change.new_version,
                        snapshot_data.len()
                    );

                    // Reset state (new epoch begins)
                    self.batch_count = 0;
                    self.last_snapshot = Instant::now();
                }
                Some(changeset) = changeset_rx.recv() => {
                    self.buffer_size += changeset.data.len();
                    self.changesets.push(changeset);

                    if self.should_flush() {
                        self.flush(&tx).await?;
                    }
                }
                _ = flush_timer.tick() => {
                    if !self.changesets.is_empty() {
                        self.flush(&tx).await?;
                    }
                }
            }
        }
    }

    fn should_flush(&self) -> bool {
        self.buffer_size >= self.config.max_batch_size ||
        self.last_flush.elapsed() >= self.config.max_batch_age
    }

    fn should_snapshot(&self) -> bool {
        self.last_snapshot.elapsed() >= self.config.snapshot_interval ||
        self.batch_count >= self.config.snapshot_threshold
    }

    async fn flush(&mut self, tx: &Sender<BatchPayload>) -> Result<()> {
        // Decide whether to send changesets or create snapshot
        if self.should_snapshot() {
            // Create full snapshot (recovery point for validators)
            let snapshot = self.create_snapshot().await?;
            tx.send(BatchPayload::Snapshot {
                data: snapshot,
                sequence: self.next_sequence(),
                timestamp: SystemTime::now(),
            }).await?;

            self.last_snapshot = Instant::now();
            self.batch_count = 0;
        } else {
            // Send incremental changesets (deterministic)
            let payload = BatchPayload::ChangesetBatch {
                changesets: self.changesets.drain(..).collect(),
                sequence: self.next_sequence(),
                timestamp: SystemTime::now(),
            };
            tx.send(payload).await?;
            self.batch_count += 1;
        }

        self.buffer_size = 0;
        self.last_flush = Instant::now();
        Ok(())
    }

    async fn create_snapshot(&self) -> Result<Vec<u8>> {
        // Use rusqlite backup API for consistent snapshot
        let conn = Connection::open(&self.db_path)?;
        let snapshot_path = format!("/tmp/snapshot-{}.db", self.sequence.load(Ordering::SeqCst));

        // Create backup
        let mut backup = conn.backup(
            rusqlite::DatabaseName::Main,
            Path::new(&snapshot_path),
            None
        )?;
        backup.step(-1)?; // Copy entire database

        // Read snapshot file
        let snapshot_data = tokio::fs::read(&snapshot_path).await?;
        tokio::fs::remove_file(&snapshot_path).await?;

        info!("Created database snapshot: {} bytes", snapshot_data.len());
        Ok(snapshot_data)
    }

    fn next_sequence(&self) -> u64 {
        self.sequence.fetch_add(1, Ordering::SeqCst)
    }
}
```

### 3. Attestor (Compression + Signing)

The Attestor service compresses batches and signs them with TEE-protected Ethereum keys.
Following cryptographic best practices, we **compress-then-sign** to prevent signature malleability
and ensure validators verify authentic compressed data.

```rust
// src/attestor/mod.rs
use k256::ecdsa::{SigningKey, Signature, signature::Signer};
use zstd;

pub struct Attestor {
    key_manager: KeyManager,
    compression_level: i32,  // Zstd level 3 (default, auto-detected by validators)
}

#[derive(Serialize, Deserialize)]
pub struct AttestedPayload {
    pub payload_type: PayloadType,
    pub compressed_data: Vec<u8>,
    pub signature: Vec<u8>,
    pub public_key: Vec<u8>,
    pub sequence: u64,
    pub checksum: Blake3Hash,
    pub attestation_token: Option<String>,  // Fresh GCP attestation token
}

#[derive(Serialize, Deserialize)]
pub enum PayloadType {
    SnapshotWithSchemaChange,
    ChangesetBatch,
    Snapshot,
}

impl Attestor {
    pub async fn run(self, rx: Receiver<BatchPayload>, tx: Sender<AttestedPayload>) {
        while let Some(payload) = rx.recv().await {
            let attested = match payload {
                BatchPayload::SnapshotWithSchemaChange { snapshot_data, schema_change, sequence, .. } => {
                    self.attest_snapshot_with_schema_change(snapshot_data, schema_change, sequence).await?
                }
                BatchPayload::ChangesetBatch { changesets, sequence, .. } => {
                    self.attest_changeset_batch(changesets, sequence).await?
                }
                BatchPayload::Snapshot { data, sequence, .. } => {
                    self.attest_snapshot(data, sequence).await?
                }
            };

            tx.send(attested).await?;
        }
    }

    async fn attest_snapshot_with_schema_change(
        &self,
        snapshot_data: Vec<u8>,
        schema_change: SchemaChange,
        sequence: u64
    ) -> Result<AttestedPayload> {
        // Combine snapshot + schema metadata
        #[derive(Serialize)]
        struct SnapshotWithSchema {
            snapshot: Vec<u8>,
            schema_change: SchemaChange,
        }

        let combined = SnapshotWithSchema {
            snapshot: snapshot_data.clone(),
            schema_change: schema_change.clone(),
        };

        let serialized = bincode::serialize(&combined)?;
        let compressed = zstd::encode_all(&serialized[..], self.compression_level)?;
        let checksum = blake3::hash(&compressed);
        let signature = self.key_manager.sign(&compressed)?;
        let attestation_token = self.key_manager.get_attestation_token().await?;

        warn!(
            "Attested SNAPSHOT WITH SCHEMA CHANGE v{} -> v{}: {} bytes snapshot, {} DDL statements",
            schema_change.old_version,
            schema_change.new_version,
            snapshot_data.len(),
            schema_change.ddl_statements.len()
        );

        Ok(AttestedPayload {
            payload_type: PayloadType::SnapshotWithSchemaChange,
            compressed_data: compressed,
            signature: signature.to_vec(),
            public_key: self.key_manager.public_key_bytes(),
            sequence,
            checksum,
            attestation_token: Some(attestation_token),
        })
    }

    async fn attest_changeset_batch(&self, changesets: Vec<Changeset>, sequence: u64) -> Result<AttestedPayload> {
        // 1. Serialize changesets
        let serialized = bincode::serialize(&changesets)?;

        // 2. Compress with Zstd level 3 (good balance of speed/ratio)
        //    Validators auto-detect this via magic bytes (0x28 0xB5 0x2F 0xFD)
        let compressed = zstd::encode_all(&serialized[..], self.compression_level)?;
        let checksum = blake3::hash(&compressed);

        // 3. Sign compressed data (compress-then-sign prevents malleability)
        let signature = self.key_manager.sign(&compressed)?;

        // 4. Get fresh attestation token from GCP metadata service
        let attestation_token = self.key_manager.get_attestation_token().await?;

        info!(
            "Attested changeset batch: {} changesets, {} ops, {} bytes compressed",
            changesets.len(),
            changesets.iter().map(|c| c.operation_count).sum::<usize>(),
            compressed.len()
        );

        Ok(AttestedPayload {
            payload_type: PayloadType::ChangesetBatch,
            compressed_data: compressed,
            signature: signature.to_vec(),
            public_key: self.key_manager.public_key_bytes(),
            sequence,
            checksum,
            attestation_token: Some(attestation_token),
        })
    }

    async fn attest_snapshot(&self, data: Vec<u8>, sequence: u64) -> Result<AttestedPayload> {
        // Snapshots are raw database bytes
        let compressed = zstd::encode_all(&data[..], self.compression_level)?;
        let checksum = blake3::hash(&compressed);
        let signature = self.key_manager.sign(&compressed)?;
        let attestation_token = self.key_manager.get_attestation_token().await?;

        info!(
            "Attested snapshot: {} bytes raw, {} bytes compressed ({:.1}% ratio)",
            data.len(),
            compressed.len(),
            (compressed.len() as f64 / data.len() as f64) * 100.0
        );

        Ok(AttestedPayload {
            payload_type: PayloadType::Snapshot,
            compressed_data: compressed,
            signature: signature.to_vec(),
            public_key: self.key_manager.public_key_bytes(),
            sequence,
            checksum,
            attestation_token: Some(attestation_token),
        })
    }
}
```

### 4. Multi-DA Publisher

Publishes attested payloads to multiple DA layers in parallel with retry logic.
The Publisher waits for successful DA publication before acknowledging, ensuring reliable delivery.

```rust
// src/publish/mod.rs
pub struct Publisher {
    publishers: Vec<Box<dyn DaPublisher>>,
    retry_policy: RetryPolicy,
    manifest_store: ManifestStore,
}

#[async_trait]
pub trait DaPublisher: Send + Sync {
    async fn publish(&self, payload: &AttestedPayload) -> Result<PublishReceipt>;
    fn name(&self) -> &str;
}

impl Publisher {
    pub async fn run(self, rx: Receiver<AttestedPayload>) {
        while let Some(payload) = rx.recv().await {
            // Block until successfully published to all configured DA layers
            let results = self.publish_all(payload).await;

            // Record manifest (sequence -> DA locations mapping)
            self.manifest_store.record(results).await?;

            // Only after successful publication do we process next batch
            // This ensures reliable delivery and prevents data loss
        }
    }

    async fn publish_all(&self, payload: AttestedPayload) -> Vec<PublishResult> {
        // Publish to all DA layers in parallel
        let futures = self.publishers.iter().map(|publisher| {
            self.publish_with_retry(publisher, &payload)
        });

        futures::future::join_all(futures).await
    }

    async fn publish_with_retry(&self, publisher: &dyn DaPublisher, payload: &AttestedPayload) -> PublishResult {
        let backoff = ExponentialBackoff {
            max_elapsed_time: Some(Duration::from_secs(300)),  // 5 min max retry
            ..Default::default()
        };

        backoff::future::retry(backoff, || async {
            publisher.publish(payload).await
                .map_err(|e| {
                    if e.is_transient() {
                        backoff::Error::Transient(e)
                    } else {
                        backoff::Error::Permanent(e)
                    }
                })
        }).await
    }
}
```

### 5. DA Layer Implementations

Each DA layer has its own implementation:

```rust
// src/publish/celestia.rs
pub struct CelestiaPublisher {
    client: CelestiaClient,
    namespace: Namespace,
    gas_price: U256,
}

#[async_trait]
impl DaPublisher for CelestiaPublisher {
    async fn publish(&self, payload: &PublishPayload) -> Result<PublishReceipt> {
        let blob = match payload {
            PublishPayload::Diff(diff) => {
                Blob::new(self.namespace, diff.data.clone())?
            }
            PublishPayload::Snapshot(snapshot) => {
                // Split large snapshots into multiple blobs
                let chunks = snapshot.data.chunks(MAX_BLOB_SIZE);
                let blobs: Vec<Blob> = chunks.map(|chunk| {
                    Blob::new(self.namespace, chunk.to_vec())
                }).collect();

                // Publish all chunks
                for blob in blobs {
                    self.client.submit_blob(blob, self.gas_price).await?;
                }
            }
        };

        let tx_hash = self.client.submit_blob(blob, self.gas_price).await?;

        Ok(PublishReceipt {
            da_layer: "celestia",
            tx_hash: tx_hash.to_string(),
            timestamp: SystemTime::now(),
            sequence: payload.sequence(),
        })
    }
}

// src/publish/ipfs.rs
pub struct IpfsPublisher {
    client: IpfsClient,
    pin_remote: bool,
    pin_service: Option<PinataClient>,
}

#[async_trait]
impl DaPublisher for IpfsPublisher {
    async fn publish(&self, payload: &PublishPayload) -> Result<PublishReceipt> {
        let data = payload.as_bytes();

        // Add to local IPFS
        let cid = self.client.add(data).await?;

        // Pin to remote service for persistence
        if self.pin_remote {
            if let Some(pin_service) = &self.pin_service {
                pin_service.pin(cid.clone()).await?;
            }
        }

        Ok(PublishReceipt {
            da_layer: "ipfs",
            tx_hash: cid.to_string(),
            timestamp: SystemTime::now(),
            sequence: payload.sequence(),
        })
    }
}
```

## Deposit Handling - Blockchain to Application Bridge

The sequencer monitors blockchain events and provides a simple HTTP API for applications to receive deposit notifications. Since the sequencer and application run as separate containers on the same physical machine, we use a localhost HTTP interface for maximum developer simplicity.

### Architecture

```
┌──────────────────────────────────────────────────────────────────┐
│                         Blockchain (L1/L2)                       │
│                    Bridge Contract (Deposits)                    │
└──────────────────────────────────────────────────────────────────┘
                                 │
                          Event Monitoring
                                 ↓
┌──────────────────────────────────────────────────────────────────┐
│                      synddb-sequencer Container                  │
│                                                                   │
│  ┌────────────────────┐    ┌─────────────────────────┐         │
│  │  Chain Monitor     │───▶│   Deposit Queue         │         │
│  │  - Watch events    │    │   - Buffer deposits     │         │
│  │  - Handle reorgs   │    │   - Track confirmations │         │
│  └────────────────────┘    └─────────────────────────┘         │
│                                         │                        │
│                                         ▼                        │
│                            ┌─────────────────────────┐          │
│                            │   Deposit HTTP API      │          │
│                            │   localhost:8432        │          │
│                            └─────────────────────────┘          │
└──────────────────────────────────────────────────────────────────┘
                                         │
                               localhost HTTP
                                         ▼
┌──────────────────────────────────────────────────────────────────┐
│                      Application Container                       │
│                                                                   │
│  ┌────────────────────────────────────────────────────────┐     │
│  │  Simple Integration:                                    │     │
│  │  - EventSource for real-time: /deposits/stream         │     │
│  │  - REST polling fallback: GET /deposits?after_id=123   │     │
│  │  - No blockchain libraries needed                      │     │
│  └────────────────────────────────────────────────────────┘     │
└──────────────────────────────────────────────────────────────────┘
```

### Deposit API Specification

The sequencer exposes a simple HTTP server on localhost port 8432 that provides deposit information to the application. This API is designed to be as simple as possible for developers familiar with REST and Server-Sent Events (SSE).

#### 1. Real-time Deposit Stream (SSE)

**Endpoint:** `GET http://localhost:8432/deposits/stream`

Returns a Server-Sent Events stream of new deposits as they are confirmed on-chain.

**Example Response (SSE format):**
```
data: {"id":1,"from":"0x742d35Cc6634C0532925a3b844Bc9e7595f0bEb3","amount":"1000000","token":"USDC","txHash":"0xabc...","blockNumber":19234567,"confirmations":12,"timestamp":"2024-01-15T10:30:00Z"}

data: {"id":2,"from":"0x5aAeb6053f3E94C9b9A09f33669435E7Ef1BeAed","amount":"500000","token":"USDC","txHash":"0xdef...","blockNumber":19234568,"confirmations":12,"timestamp":"2024-01-15T10:31:00Z"}
```

**Client Example (JavaScript):**
```javascript
const events = new EventSource('http://localhost:8432/deposits/stream');

events.onmessage = (event) => {
  const deposit = JSON.parse(event.data);
  console.log(`New deposit: ${deposit.amount} ${deposit.token} from ${deposit.from}`);

  // Process deposit in application
  await processDeposit(deposit);
};

events.onerror = (error) => {
  console.error('SSE connection error:', error);
  // EventSource will auto-reconnect
};
```

#### 2. Deposit Queue (REST Polling)

**Endpoint:** `GET http://localhost:8432/deposits?after_id={last_processed_id}`

Returns all deposits after the specified ID. Used for polling or recovery after disconnection.

**Query Parameters:**
- `after_id` (optional): Return only deposits with ID greater than this value
- `limit` (optional): Maximum number of deposits to return (default: 100, max: 1000)

**Example Response:**
```json
{
  "deposits": [
    {
      "id": 124,
      "from": "0x742d35Cc6634C0532925a3b844Bc9e7595f0bEb3",
      "amount": "1000000",
      "token": "USDC",
      "txHash": "0xabc123...",
      "blockNumber": 19234567,
      "confirmations": 12,
      "timestamp": "2024-01-15T10:30:00Z"
    },
    {
      "id": 125,
      "from": "0x5aAeb6053f3E94C9b9A09f33669435E7Ef1BeAed",
      "amount": "500000",
      "token": "USDC",
      "txHash": "0xdef456...",
      "blockNumber": 19234568,
      "confirmations": 12,
      "timestamp": "2024-01-15T10:31:00Z"
    }
  ],
  "hasMore": false,
  "latestId": 125
}
```

**Client Example (Python):**
```python
import requests
import time

last_id = 0

while True:
    response = requests.get(f'http://localhost:8432/deposits?after_id={last_id}')
    data = response.json()

    for deposit in data['deposits']:
        process_deposit(deposit)
        last_id = deposit['id']

    time.sleep(5)  # Poll every 5 seconds
```

#### 3. Acknowledge Deposit (Optional)

**Endpoint:** `POST http://localhost:8432/deposits/{id}/ack`

Optionally acknowledge that a deposit has been processed. This is for monitoring/debugging purposes only and doesn't affect deposit delivery.

**Request Body:**
```json
{
  "processed": true,
  "processingNote": "Credited to user account"  // Optional
}
```

**Response:**
```json
{
  "success": true,
  "deposit_id": 124
}
```

### Implementation

#### Chain Monitor Component

```rust
// src/deposits/chain_monitor.rs
use ethers::prelude::*;
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct ChainMonitor {
    provider: Arc<Provider<Ws>>,
    bridge_contract: Address,
    deposit_queue: Arc<Mutex<DepositQueue>>,
    confirmation_blocks: u64,
}

impl ChainMonitor {
    pub async fn new(
        rpc_url: &str,
        bridge_address: &str,
        deposit_queue: Arc<Mutex<DepositQueue>>,
        confirmation_blocks: u64,
    ) -> Result<Self> {
        let provider = Provider::<Ws>::connect(rpc_url).await?;
        let bridge_contract = bridge_address.parse::<Address>()?;

        Ok(Self {
            provider: Arc::new(provider),
            bridge_contract,
            deposit_queue,
            confirmation_blocks,
        })
    }

    pub async fn start_monitoring(self) -> Result<()> {
        // Define the Deposit event ABI
        let deposit_event = Event {
            name: "Deposit".to_string(),
            inputs: vec![
                EventParam {
                    name: "from".to_string(),
                    kind: ParamType::Address,
                    indexed: true,
                },
                EventParam {
                    name: "amount".to_string(),
                    kind: ParamType::Uint(256),
                    indexed: false,
                },
                EventParam {
                    name: "token".to_string(),
                    kind: ParamType::Address,
                    indexed: true,
                },
            ],
            anonymous: false,
        };

        // Subscribe to Deposit events
        let filter = Filter::new()
            .address(self.bridge_contract)
            .event(&deposit_event);

        let mut stream = self.provider.subscribe_logs(&filter).await?;

        // Process events as they arrive
        while let Some(log) = stream.next().await {
            match self.process_deposit_event(log).await {
                Ok(_) => {},
                Err(e) => error!("Failed to process deposit: {}", e),
            }
        }

        Ok(())
    }

    async fn process_deposit_event(&self, log: Log) -> Result<()> {
        // Wait for confirmations
        let current_block = self.provider.get_block_number().await?;
        let confirmations = current_block.saturating_sub(log.block_number.unwrap_or_default());

        if confirmations < self.confirmation_blocks {
            // Not enough confirmations yet, re-queue for later
            tokio::time::sleep(Duration::from_secs(12)).await;
            return self.process_deposit_event(log).await;
        }

        // Parse deposit data from log
        let deposit = Deposit {
            id: self.deposit_queue.lock().await.next_id(),
            from: format!("0x{:x}", log.topics[1]),  // First indexed param
            amount: U256::from_big_endian(&log.data[0..32]).to_string(),
            token: self.get_token_symbol(log.topics[2]).await?,
            tx_hash: format!("0x{:x}", log.transaction_hash.unwrap_or_default()),
            block_number: log.block_number.unwrap_or_default().as_u64(),
            confirmations: confirmations.as_u64(),
            timestamp: Utc::now(),
        };

        // Add to queue
        self.deposit_queue.lock().await.add_deposit(deposit)?;

        Ok(())
    }

    async fn get_token_symbol(&self, token_address: H256) -> Result<String> {
        // In production, query token contract for symbol
        // For now, use a simple mapping
        match format!("{:x}", token_address).as_str() {
            "a0b86991c6218b36c1d19d4a2e9eb0ce3606eb48" => Ok("USDC".to_string()),
            "dac17f958d2ee523a2206206994597c13d831ec7" => Ok("USDT".to_string()),
            _ => Ok("UNKNOWN".to_string()),
        }
    }
}
```

#### Deposit Queue

```rust
// src/deposits/queue.rs
use std::collections::VecDeque;
use chrono::{DateTime, Utc};
use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Deposit {
    pub id: u64,
    pub from: String,
    pub amount: String,
    pub token: String,
    pub tx_hash: String,
    pub block_number: u64,
    pub confirmations: u64,
    pub timestamp: DateTime<Utc>,
}

pub struct DepositQueue {
    deposits: VecDeque<Deposit>,
    next_id: AtomicU64,
    max_queue_size: usize,
    retention_period: Duration,
}

impl DepositQueue {
    pub fn new(max_queue_size: usize, retention_period: Duration) -> Self {
        Self {
            deposits: VecDeque::new(),
            next_id: AtomicU64::new(1),
            max_queue_size,
            retention_period,
        }
    }

    pub fn add_deposit(&mut self, mut deposit: Deposit) -> Result<()> {
        deposit.id = self.next_id.fetch_add(1, Ordering::SeqCst);

        // Remove old deposits beyond retention period
        let cutoff = Utc::now() - self.retention_period;
        self.deposits.retain(|d| d.timestamp > cutoff);

        // Ensure queue doesn't grow unbounded
        if self.deposits.len() >= self.max_queue_size {
            self.deposits.pop_front();
        }

        self.deposits.push_back(deposit.clone());

        info!("New deposit queued: {} {} from {} (id: {})",
              deposit.amount, deposit.token, deposit.from, deposit.id);

        Ok(())
    }

    pub fn get_deposits_after(&self, after_id: u64, limit: usize) -> Vec<Deposit> {
        self.deposits
            .iter()
            .filter(|d| d.id > after_id)
            .take(limit)
            .cloned()
            .collect()
    }

    pub fn get_latest_deposits(&self, count: usize) -> Vec<Deposit> {
        self.deposits
            .iter()
            .rev()
            .take(count)
            .rev()
            .cloned()
            .collect()
    }

    pub fn next_id(&self) -> u64 {
        self.next_id.load(Ordering::SeqCst)
    }
}
```

#### HTTP API Server

```rust
// src/deposits/api.rs
use axum::{
    extract::{Query, Path, State},
    response::sse::{Event, Sse},
    Json, Router,
};
use tokio_stream::StreamExt;
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct DepositApi {
    deposit_queue: Arc<Mutex<DepositQueue>>,
    sse_broadcaster: Arc<Mutex<SseBroadcaster>>,
}

impl DepositApi {
    pub fn router(deposit_queue: Arc<Mutex<DepositQueue>>) -> Router {
        let sse_broadcaster = Arc::new(Mutex::new(SseBroadcaster::new()));

        let api = Arc::new(Self {
            deposit_queue: deposit_queue.clone(),
            sse_broadcaster: sse_broadcaster.clone(),
        });

        Router::new()
            .route("/deposits/stream", get(Self::deposit_stream))
            .route("/deposits", get(Self::get_deposits))
            .route("/deposits/:id/ack", post(Self::acknowledge_deposit))
            .with_state(api)
    }

    // SSE endpoint for real-time deposits
    async fn deposit_stream(
        State(api): State<Arc<DepositApi>>,
    ) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
        let receiver = api.sse_broadcaster.lock().await.subscribe();

        let stream = BroadcastStream::new(receiver).map(|msg| {
            match msg {
                Ok(deposit) => {
                    let data = serde_json::to_string(&deposit).unwrap_or_default();
                    Ok(Event::default().data(data))
                }
                Err(_) => Ok(Event::default().comment("error")),
            }
        });

        Sse::new(stream)
            .keep_alive(Duration::from_secs(30))
    }

    // REST endpoint for polling/recovery
    async fn get_deposits(
        State(api): State<Arc<DepositApi>>,
        Query(params): Query<GetDepositsParams>,
    ) -> Json<GetDepositsResponse> {
        let queue = api.deposit_queue.lock().await;

        let after_id = params.after_id.unwrap_or(0);
        let limit = params.limit.unwrap_or(100).min(1000);

        let deposits = queue.get_deposits_after(after_id, limit);
        let has_more = deposits.len() >= limit;
        let latest_id = deposits.last().map(|d| d.id).unwrap_or(after_id);

        Json(GetDepositsResponse {
            deposits,
            has_more,
            latest_id,
        })
    }

    // Optional acknowledgment endpoint
    async fn acknowledge_deposit(
        State(api): State<Arc<DepositApi>>,
        Path(id): Path<u64>,
        Json(body): Json<AckRequest>,
    ) -> Json<AckResponse> {
        info!("Deposit {} acknowledged: processed={}, note={:?}",
              id, body.processed, body.processing_note);

        Json(AckResponse {
            success: true,
            deposit_id: id,
        })
    }
}

#[derive(Deserialize)]
struct GetDepositsParams {
    after_id: Option<u64>,
    limit: Option<usize>,
}

#[derive(Serialize)]
struct GetDepositsResponse {
    deposits: Vec<Deposit>,
    has_more: bool,
    latest_id: u64,
}

#[derive(Deserialize)]
struct AckRequest {
    processed: bool,
    processing_note: Option<String>,
}

#[derive(Serialize)]
struct AckResponse {
    success: bool,
    deposit_id: u64,
}

// SSE broadcaster for pushing deposits to all connected clients
struct SseBroadcaster {
    clients: Vec<tokio::sync::broadcast::Sender<Deposit>>,
}

impl SseBroadcaster {
    fn new() -> Self {
        Self {
            clients: Vec::new(),
        }
    }

    fn subscribe(&mut self) -> tokio::sync::broadcast::Receiver<Deposit> {
        let (tx, rx) = tokio::sync::broadcast::channel(100);
        self.clients.push(tx);
        rx
    }

    async fn broadcast(&self, deposit: Deposit) {
        for client in &self.clients {
            let _ = client.send(deposit.clone());
        }
    }
}
```

### Configuration

Add to the sequencer configuration file:

```yaml
# Deposit monitoring configuration
deposits:
  # Blockchain monitoring
  chain:
    enabled: true
    rpc_url: "wss://eth-mainnet.g.alchemy.com/v2/${ALCHEMY_API_KEY}"
    bridge_contract: "0x..."  # Bridge contract address
    confirmation_blocks: 12    # Wait for 12 confirmations
    poll_interval_secs: 12     # Check every 12 seconds (1 block)

  # HTTP API for applications
  api:
    enabled: true
    host: "127.0.0.1"         # localhost only (same machine)
    port: 8432                # Deposit API port

  # Queue management
  queue:
    max_size: 10000           # Maximum deposits in queue
    retention_hours: 24       # Keep deposits for 24 hours

  # Monitoring
  monitoring:
    log_level: "info"
    metrics: true
```

### Integration Examples

#### Node.js Application

```javascript
// Simple deposit handler for Node.js applications
class DepositHandler {
  constructor() {
    this.lastProcessedId = this.loadLastProcessedId();
    this.eventSource = null;
  }

  async start() {
    // Try SSE first for real-time updates
    this.connectSSE();

    // Also poll periodically as backup
    setInterval(() => this.pollDeposits(), 30000);
  }

  connectSSE() {
    this.eventSource = new EventSource('http://localhost:8432/deposits/stream');

    this.eventSource.onmessage = async (event) => {
      const deposit = JSON.parse(event.data);
      await this.processDeposit(deposit);
    };

    this.eventSource.onerror = (error) => {
      console.error('SSE error, will reconnect:', error);
      // EventSource auto-reconnects
    };
  }

  async pollDeposits() {
    try {
      const response = await fetch(
        `http://localhost:8432/deposits?after_id=${this.lastProcessedId}`
      );
      const data = await response.json();

      for (const deposit of data.deposits) {
        await this.processDeposit(deposit);
      }
    } catch (error) {
      console.error('Polling error:', error);
    }
  }

  async processDeposit(deposit) {
    console.log(`Processing deposit ${deposit.id}: ${deposit.amount} ${deposit.token}`);

    // Update user balance in database
    await db.query(
      'UPDATE user_balances SET amount = amount + ? WHERE address = ? AND token = ?',
      [deposit.amount, deposit.from, deposit.token]
    );

    this.lastProcessedId = deposit.id;
    this.saveLastProcessedId(deposit.id);

    // Optional: acknowledge processing
    await fetch(`http://localhost:8432/deposits/${deposit.id}/ack`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ processed: true })
    });
  }
}
```

#### Python Application

```python
import asyncio
import aiohttp
import json
from aiohttp_sse_client import client as sse_client

class DepositHandler:
    def __init__(self):
        self.base_url = "http://localhost:8432"
        self.last_processed_id = self.load_last_processed_id()

    async def start(self):
        # Start both SSE and polling concurrently
        await asyncio.gather(
            self.listen_sse(),
            self.poll_periodically()
        )

    async def listen_sse(self):
        async with sse_client.EventSource(
            f'{self.base_url}/deposits/stream'
        ) as event_source:
            async for event in event_source:
                if event.data:
                    deposit = json.loads(event.data)
                    await self.process_deposit(deposit)

    async def poll_periodically(self):
        while True:
            await asyncio.sleep(30)  # Poll every 30 seconds
            await self.poll_deposits()

    async def poll_deposits(self):
        async with aiohttp.ClientSession() as session:
            async with session.get(
                f'{self.base_url}/deposits',
                params={'after_id': self.last_processed_id}
            ) as response:
                data = await response.json()
                for deposit in data['deposits']:
                    await self.process_deposit(deposit)

    async def process_deposit(self, deposit):
        print(f"Processing deposit {deposit['id']}: "
              f"{deposit['amount']} {deposit['token']} from {deposit['from']}")

        # Update application state
        # ...

        self.last_processed_id = deposit['id']
        self.save_last_processed_id(deposit['id'])
```

### Benefits of This Architecture

1. **Simple for Developers**: Just HTTP/REST and SSE - technologies every developer knows
2. **No Blockchain Complexity**: Applications don't need web3 libraries or blockchain knowledge
3. **Reliable**: Dual mechanism (SSE + polling) ensures no deposits are missed
4. **Fast**: localhost communication has microsecond latency
5. **Testable**: Easy to mock HTTP endpoints for testing
6. **Language Agnostic**: Works with any programming language that has HTTP support
7. **Monitoring Friendly**: Clear HTTP endpoints make debugging and monitoring straightforward

The sequencer handles all blockchain complexity (reorgs, confirmations, event parsing) and presents a clean, simple interface that any application developer can integrate with minimal effort.

## Configuration

```yaml
# config/default.yaml
database:
  path: "/data/app.db"
  wal_mode: true

monitor:
  poll_interval_ms: 100
  page_cache_size: 1000

batch:
  max_batch_size: 1048576 # 1MB
  max_batch_age_ms: 1000 # 1 second
  optimize_sql: true

compression:
  strategy: "zstd"
  zstd_level: 3
  snapshot_interval_mins: 60
  snapshot_threshold_diffs: 1000

publishers:
  celestia:
    enabled: true
    endpoint: "https://rpc.celestia.org"
    namespace: "0x00000000000000000000000000000000synddb"
    gas_price: "0.01"
    retry_attempts: 3

  eigenda:
    enabled: false
    endpoint: "https://disperser.eigenda.xyz"
    quorum_threshold: 67

  ipfs:
    enabled: true
    endpoint: "http://localhost:5001"
    pin_remote: true
    pinata_api_key: "${PINATA_API_KEY}"

  arweave:
    enabled: false
    endpoint: "https://arweave.net"
    wallet_path: "/secrets/arweave-wallet.json"

tee:
  enabled: false
  attestation_type: "dcap" # or "epid"
  mrenclave: "0x..."
  mrsigner: "0x..."

metrics:
  enabled: true
  port: 9090

logging:
  level: "info"
  format: "json"
```

## TEE Integration with GCP Confidential Space

The sequencer leverages GCP Confidential Space for hardware-protected Ethereum key management.
The sequencer runs in a **separate container** from the application within the same Confidential Space VM,
providing strong isolation while maintaining filesystem access to the SQLite database.

### Security Model: Same-VM, Separate Containers

```
┌──────────────────────────────────────────────────────────────────┐
│              GCP Confidential Space VM (TEE)                     │
│  Hardware Root of Trust (AMD SEV-SNP / Intel TDX)               │
│                                                                   │
│  ┌────────────────────────┐  ┌────────────────────────┐        │
│  │   Application          │  │   synddb-sequencer       │        │
│  │   Container            │  │   Container            │        │
│  │                        │  │                        │        │
│  │  - Any language        │  │  - Read-only SQLite    │        │
│  │  - Writes to SQLite    │  │  - Ethereum keys       │        │
│  │  - NO key access       │  │  - Signs batches       │        │
│  │                        │  │  - Publishes to DA     │        │
│  └────────────────────────┘  └────────────────────────┘        │
│             │                            │                       │
│             └────────┬───────────────────┘                       │
│                      ↓                                           │
│           ┌──────────────────────┐                              │
│           │  Shared Persistent   │                              │
│           │  Disk (SQLite DB)    │                              │
│           └──────────────────────┘                              │
│                                                                   │
│  Container-level isolation prevents application from accessing   │
│  sequencer memory space where Ethereum keys are held.           │
└──────────────────────────────────────────────────────────────────┘
                     ↓ Attestation Token + Signature
┌──────────────────────────────────────────────────────────────────┐
│                        Bridge.sol                                │
│  - Verify attestation via SP1 zkVM                              │
│  - Register public key after verification                       │
│  - Track valid signers via ecrecover                            │
└──────────────────────────────────────────────────────────────────┘
```

### Why Same-VM Architecture is Secure

1. **Container Isolation**: Linux namespaces and cgroups prevent cross-container memory access
2. **Read-Only SQLite Access**: Sequencer opens DB with `SQLITE_OPEN_READ_ONLY` flag
3. **Memory Encryption**: AMD SEV-SNP encrypts all VM memory including both containers
4. **No Shared Memory**: Containers communicate only via filesystem (SQLite DB file)
5. **Principle of Least Privilege**: Application has no credentials to access Secret Manager
6. **Attestation Binding**: Keys in Secret Manager are bound to sequencer container digest only

### Key Management Implementation

```rust
// src/attestor/key_manager.rs
use gcp_auth::AuthenticationManager;
use google_cloud_secretmanager::client::{Client as SecretClient, ClientConfig};
use google_cloud_default::WithAuthExt;
use serde::{Deserialize, Serialize};
use k256::ecdsa::{SigningKey, VerifyingKey, Signature, signature::Signer};
use k256::elliptic_curve::sec1::ToEncodedPoint;
use sha2::{Sha256, Digest};
use anyhow::Result;

/// Manages Ethereum secp256k1 keys for signing batches.
/// Keys are generated in TEE and stored in GCP Secret Manager,
/// bound to the sequencer container digest via Workload Identity.
pub struct KeyManager {
    signing_key: SigningKey,
    verifying_key: VerifyingKey,
    ethereum_address: [u8; 20],
    secret_client: SecretClient,
    project_id: String,
    secret_name: String,
}

#[derive(Serialize, Deserialize)]
struct SealedKeyData {
    /// secp256k1 private key (32 bytes)
    private_key: Vec<u8>,
    /// secp256k1 public key (compressed, 33 bytes)
    public_key: Vec<u8>,
    /// Ethereum address derived from public key
    ethereum_address: [u8; 20],
    created_at: i64,
    /// Attestation token at key creation
    attestation_token: String,
}

impl KeyManager {
    pub async fn init() -> Result<Self> {
        // Get project ID from metadata service
        let project_id = Self::get_project_id().await?;

        // Initialize Secret Manager client with Workload Identity
        let config = ClientConfig::default().with_auth().await?;
        let secret_client = SecretClient::new(config).await?;

        let secret_name = "synddb-sequencer-signing-key".to_string();

        // Try to load existing key or generate new one
        let (signing_key, verifying_key, ethereum_address) =
            match Self::load_sealed_key(&secret_client, &project_id, &secret_name).await {
                Ok(key_data) => {
                    info!("Loaded existing Ethereum signing key from Secret Manager");
                    let signing_key = SigningKey::from_slice(&key_data.private_key)?;
                    let verifying_key = VerifyingKey::from_sec1_bytes(&key_data.public_key)?;
                    (signing_key, verifying_key, key_data.ethereum_address)
                }
                Err(_) => {
                    info!("Generating new Ethereum signing key (secp256k1)");

                    // Generate secp256k1 key pair
                    let signing_key = SigningKey::random(&mut rand::thread_rng());
                    let verifying_key = signing_key.verifying_key();

                    // Derive Ethereum address from public key
                    let ethereum_address = Self::derive_ethereum_address(&verifying_key);

                    // Get attestation token for this container
                    let attestation_token = Self::get_attestation_token().await?;

                    // Seal to Secret Manager (bound to container digest via WI)
                    Self::seal_key(
                        &secret_client,
                        &project_id,
                        &secret_name,
                        &signing_key,
                        &verifying_key,
                        ethereum_address,
                        &attestation_token
                    ).await?;

                    (signing_key, verifying_key, ethereum_address)
                }
            };

        Ok(Self {
            signing_key,
            verifying_key,
            ethereum_address,
            secret_client,
            project_id,
            secret_name,
        })
    }

    /// Derive Ethereum address from secp256k1 public key
    /// Address = keccak256(pubkey)[12:32]
    fn derive_ethereum_address(verifying_key: &VerifyingKey) -> [u8; 20] {
        use sha3::{Keccak256, Digest};

        // Get uncompressed public key (65 bytes: 0x04 + x + y)
        let public_key_bytes = verifying_key.to_encoded_point(false);
        let public_key_slice = &public_key_bytes.as_bytes()[1..]; // Skip 0x04 prefix

        // Hash with Keccak256
        let mut hasher = Keccak256::new();
        hasher.update(public_key_slice);
        let hash = hasher.finalize();

        // Take last 20 bytes
        let mut address = [0u8; 20];
        address.copy_from_slice(&hash[12..32]);
        address
    }

    async fn get_attestation_token() -> Result<String> {
        // Get attestation token from Confidential Space metadata service
        let client = reqwest::Client::new();

        // Custom audience for our application
        let audience = "https://synddb.io/sequencer";

        let response = client
            .get("http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token")
            .query(&[
                ("audience", audience),
                ("format", "full"),
                ("licenses", "TRUE"),  // Include container image measurements
            ])
            .header("Metadata-Flavor", "Google")
            .send()
            .await?;

        #[derive(Deserialize)]
        struct TokenResponse {
            token: String,
        }

        let token_resp: TokenResponse = response.json().await?;
        Ok(token_resp.token)
    }

    async fn seal_key(
        secret_client: &SecretClient,
        project_id: &str,
        secret_name: &str,
        signing_key: &SigningKey,
        verifying_key: &VerifyingKey,
        ethereum_address: [u8; 20],
        attestation_token: &str,
    ) -> Result<()> {
        let key_data = SealedKeyData {
            private_key: signing_key.to_bytes().to_vec(),
            public_key: verifying_key.to_encoded_point(true).as_bytes().to_vec(), // Compressed
            ethereum_address,
            created_at: chrono::Utc::now().timestamp(),
            attestation_token: attestation_token.to_string(),
        };

        let secret_data = serde_json::to_vec(&key_data)?;

        // Create secret with Workload Identity binding
        // IAM policy ensures only sequencer container with matching digest can access
        secret_client
            .create_secret(
                project_id,
                secret_name,
                secret_data,
                Some(vec![
                    ("synddb/environment", "confidential-space"),
                    ("synddb/component", "sequencer"),
                    ("synddb/key-type", "secp256k1-ethereum"),
                ]),
            )
            .await?;

        info!("Sealed Ethereum key to Secret Manager: 0x{}", hex::encode(ethereum_address));
        Ok(())
    }

    async fn load_sealed_key(
        secret_client: &SecretClient,
        project_id: &str,
        secret_name: &str,
    ) -> Result<SealedKeyData> {
        let secret_data = secret_client
            .access_secret_version(project_id, secret_name, "latest")
            .await?;

        let key_data: SealedKeyData = serde_json::from_slice(&secret_data)?;
        Ok(key_data)
    }

    /// Sign data with secp256k1 key (returns recoverable signature)
    pub fn sign(&self, data: &[u8]) -> Result<Signature> {
        use k256::ecdsa::signature::Signer;
        Ok(self.signing_key.sign(data))
    }

    /// Get public key bytes (compressed, 33 bytes)
    pub fn public_key_bytes(&self) -> Vec<u8> {
        self.verifying_key.to_encoded_point(true).as_bytes().to_vec()
    }

    /// Get Ethereum address
    pub fn ethereum_address(&self) -> [u8; 20] {
        self.ethereum_address
    }

    pub async fn get_attestation_token(&self) -> Result<String> {
        Self::get_attestation_token().await
    }

    async fn get_project_id() -> Result<String> {
        let client = reqwest::Client::new();
        let response = client
            .get("http://metadata.google.internal/computeMetadata/v1/project/project-id")
            .header("Metadata-Flavor", "Google")
            .send()
            .await?;
        Ok(response.text().await?)
    }
}
```

### Docker Configuration

```dockerfile
# Dockerfile.confidential
FROM rust:1.75 as builder

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src

# Build with Confidential Space features
RUN cargo build --release --features confidential-space

# Runtime image for Confidential Space
FROM gcr.io/confidential-space-images/base:latest

# Install required dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Copy binary
COPY --from=builder /app/target/release/synddb-sequencer /usr/local/bin/

# Create non-root user
RUN useradd -m -u 1000 synddb && \
    chown -R synddb:synddb /usr/local/bin/synddb-sequencer

USER synddb

# Health check
HEALTHCHECK --interval=30s --timeout=3s \
    CMD curl -f http://localhost:9090/health || exit 1

# Entry point with attestation initialization
ENTRYPOINT ["/usr/local/bin/synddb-sequencer"]
CMD ["--attestation", "confidential-space", "--config", "/config/sequencer.yaml"]
```

### Deployment Configuration

```yaml
# confidential-space-deployment.yaml
apiVersion: compute.cnrm.cloud.google.com/v1beta1
kind: ComputeInstance
metadata:
  name: synddb-sequencer-tee
spec:
  machineType: n2d-standard-4 # AMD SEV-SNP capable
  zone: us-central1-a

  confidentialInstanceConfig:
    enableConfidentialCompute: true
    confidentialComputeType: SEV_SNP # or TDX for Intel

  shieldedInstanceConfig:
    enableSecureBoot: true
    enableVtpm: true
    enableIntegrityMonitoring: true

  scheduling:
    onHostMaintenance: TERMINATE # Required for Confidential VMs

  serviceAccounts:
    - email: synddb-sequencer@${PROJECT_ID}.iam.gserviceaccount.com
      scopes:
        - https://www.googleapis.com/auth/cloud-platform

  metadata:
    items:
      - key: tee-container-log-redirect
        value: "true"
      - key: tee-image-reference
        value: "gcr.io/${PROJECT_ID}/synddb-sequencer:latest"
      - key: tee-restart-policy
        value: "Always"
      - key: tee-env-ATTESTATION_AUDIENCE
        value: "https://synddb.io/sequencer"
      - key: tee-env-SECRET_PROJECT_ID
        value: "${PROJECT_ID}"

  bootDisk:
    initializeParams:
      image: projects/confidential-space-images/global/images/confidential-space-release
```

### Workload Identity Configuration

```yaml
# workload-identity.yaml
apiVersion: iam.cnrm.cloud.google.com/v1beta1
kind: IAMServiceAccount
metadata:
  name: synddb-sequencer
spec:
  displayName: SyndDB Sequencer Service Account
---
apiVersion: iam.cnrm.cloud.google.com/v1beta1
kind: IAMPolicyMember
metadata:
  name: synddb-sequencer-secretmanager
spec:
  memberFrom:
    serviceAccountRef:
      name: synddb-sequencer
  role: roles/secretmanager.secretAccessor
  resourceRef:
    kind: Project
---
apiVersion: iam.cnrm.cloud.google.com/v1beta1
kind: IAMPolicy
metadata:
  name: synddb-sequencer-secret-policy
spec:
  resourceRef:
    kind: Secret
    name: synddb-sequencer-signing-key
  policy:
    bindings:
      - role: roles/secretmanager.secretAccessor
        members:
          - serviceAccount:synddb-sequencer@${PROJECT_ID}.iam.gserviceaccount.com
        condition:
          title: Only from Confidential Space
          expression: |
            assertion.sub == 'synddb-sequencer@${PROJECT_ID}.iam.gserviceaccount.com' &&
            'image_digest' in assertion &&
            assertion.image_digest == '${EXPECTED_IMAGE_DIGEST}'
```

### Configuration File

```yaml
# config/sequencer-confidential.yaml
database:
  path: "/data/app.db"
  wal_mode: true

attestation:
  enabled: true
  provider: "gcp-confidential-space"

  # GCP Confidential Space settings
  gcp:
    project_id: "${PROJECT_ID}"
    secret_name: "synddb-sequencer-signing-key"
    attestation_audience: "https://synddb.io/sequencer"

    # Expected measurements for verification
    expected_measurements:
      container_image_digest: "${EXPECTED_IMAGE_DIGEST}"

  # How often to refresh attestation
  refresh_interval_mins: 60

  # Bridge contract for key registration
  bridge_contract: "0x..."
  bridge_rpc: "https://eth-mainnet.g.alchemy.com/v2/..."

publishers:
  celestia:
    enabled: true
    endpoint: "https://rpc.celestia.org"
    # Include attestation with all published data
    include_attestation: true

monitoring:
  metrics:
    enabled: true
    port: 9090
  health:
    enabled: true
    port: 8080
```

## Additional Operational Considerations

### 1. Disaster Recovery & Persistence ⚠️ TODO

**Problem:** If sequencer crashes, in-memory changesets are lost
**Solution:** Persistent queue before DA publish
```rust
// src/publish/persistent_queue.rs
pub struct PersistentQueue {
    disk_buffer: sled::Db,  // Fast embedded key-value store
    sequence_tracker: AtomicU64,
}

impl PersistentQueue {
    // Write changeset to disk before publishing
    pub fn enqueue(&self, payload: AttestedPayload) -> Result<()> {
        self.disk_buffer.insert(
            &payload.sequence.to_be_bytes(),
            bincode::serialize(&payload)?
        )?;
        Ok(())
    }

    // Mark as published after DA confirmation
    pub fn dequeue(&self, sequence: u64) -> Result<()> {
        self.disk_buffer.remove(&sequence.to_be_bytes())?;
        Ok(())
    }

    // On restart, republish any unconfirmed payloads
    pub fn recover(&self) -> Result<Vec<AttestedPayload>> {
        let mut pending = Vec::new();
        for item in self.disk_buffer.iter() {
            let (_, payload_bytes) = item?;
            pending.push(bincode::deserialize(&payload_bytes)?);
        }
        Ok(pending)
    }
}
```

**Status:** ⚠️ TODO - Add persistent queue between Attestor and Publisher

### 2. Backpressure & Memory Management ⚠️ TODO

**Problem:** Application writes faster than sequencer can publish
**Solution:** Bounded channels with monitoring
```rust
// Bounded channels prevent memory exhaustion
let (changeset_tx, changeset_rx) = mpsc::channel::<Changeset>(1000);
let (batch_tx, batch_rx) = mpsc::channel::<BatchPayload>(100);

// Monitor queue depth
if changeset_tx.capacity() < 100 {
    warn!("Sequencer falling behind: {} changesets queued",
          1000 - changeset_tx.capacity());
}
```

**Status:** ⚠️ TODO - Add queue depth monitoring and alerts

### 3. Graceful Shutdown ⚠️ TODO

**Problem:** Sequencer shutdown could lose in-flight changesets
**Solution:** Flush all pending work before exit
```rust
impl Sequencer {
    pub async fn shutdown(&mut self) -> Result<()> {
        info!("Graceful shutdown initiated");

        // 1. Stop accepting new changesets
        self.session_monitor.stop().await?;

        // 2. Flush all pending batches
        self.batcher.flush_all(&self.attestor_tx).await?;

        // 3. Wait for attestor to finish
        self.attestor.wait_idle().await?;

        // 4. Wait for all DA publishes to confirm
        self.publisher.wait_for_pending().await?;

        // 5. Persist state checkpoint
        self.save_checkpoint().await?;

        info!("Graceful shutdown complete");
        Ok(())
    }

    async fn save_checkpoint(&self) -> Result<()> {
        let state = SequencerState {
            last_published_sequence: self.sequence.load(Ordering::SeqCst),
            last_schema_version: self.schema_version,
        };
        fs::write("/data/sequencer.checkpoint", serde_json::to_vec(&state)?)?;
        Ok(())
    }
}
```

**Status:** ⚠️ TODO - Implement graceful shutdown

### 4. Large Transaction Handling ⚠️ TODO

**Problem:** Single transaction with millions of rows creates huge changeset
**Solution:** Detect large changesets and force snapshot instead
```rust
impl Batcher {
    const MAX_CHANGESET_SIZE: usize = 100_000_000; // 100MB

    async fn flush(&mut self, tx: &Sender<BatchPayload>) -> Result<()> {
        let batch_size: usize = self.changesets.iter()
            .map(|c| c.data.len())
            .sum();

        if batch_size > Self::MAX_CHANGESET_SIZE {
            warn!("Large transaction detected ({} bytes), forcing snapshot", batch_size);

            // Discard changesets, create snapshot instead
            self.changesets.clear();
            let snapshot_data = self.create_snapshot().await?;

            tx.send(BatchPayload::Snapshot {
                data: snapshot_data,
                sequence: self.next_sequence(),
                timestamp: SystemTime::now(),
            }).await?;
        } else {
            // Normal changeset batch
            // ...
        }
    }
}
```

**Status:** ⚠️ TODO - Add large transaction detection

### 5. Key Rotation ⚠️ TODO

**Problem:** Ethereum signing keys may need rotation for security
**Solution:** Publish key rotation messages to validators
```rust
pub struct KeyRotation {
    old_public_key: Vec<u8>,
    new_public_key: Vec<u8>,
    rotation_sequence: u64,
    attestation_token: String,  // Proves new key is from same TEE
}

impl KeyManager {
    pub async fn rotate_key(&mut self) -> Result<KeyRotation> {
        let old_key = self.verifying_key.clone();

        // Generate new key
        let new_signing_key = SigningKey::random(&mut rand::thread_rng());
        let new_verifying_key = new_signing_key.verifying_key();

        // Get attestation for new key
        let attestation = self.get_attestation_token().await?;

        // Seal new key to Secret Manager
        self.seal_key(&new_signing_key, &new_verifying_key, &attestation).await?;

        // Keep both keys active during transition
        self.old_signing_key = Some(self.signing_key);
        self.signing_key = new_signing_key;

        Ok(KeyRotation {
            old_public_key: old_key.to_encoded_point(true).as_bytes().to_vec(),
            new_public_key: new_verifying_key.to_encoded_point(true).as_bytes().to_vec(),
            rotation_sequence: self.current_sequence(),
            attestation_token: attestation,
        })
    }
}
```

**Status:** ⚠️ TODO - Implement key rotation protocol

### 6. Observability & Monitoring ⚠️ TODO

**Critical Metrics:**
```rust
// GCP Cloud Logging structured events
pub struct SequencerMetrics {
    // Lag metrics
    pub changeset_lag_seconds: f64,        // Time from commit to publish
    pub queue_depth: usize,                 // Unpublished changesets

    // Throughput metrics
    pub changesets_per_second: f64,
    pub bytes_published_per_second: f64,

    // Error metrics
    pub da_publish_failures: u64,
    pub schema_detection_errors: u64,

    // Health metrics
    pub session_extension_healthy: bool,
    pub db_connection_healthy: bool,
    pub da_layers_reachable: HashMap<String, bool>,
}
```

**Status:** ⚠️ TODO - Add comprehensive metrics

### 7. Supported SQLite Features

**Known Limitations:**
- ❌ Memory databases (`:memory:`) - Cannot persist
- ❌ Temporary tables - Not replicated by Session Extension
- ⚠️ ATTACH DATABASE - Each database needs separate sequencer
- ⚠️ Encrypted databases (SQLCipher) - Need special handling
- ⚠️ Custom collations - Must match on validators

**Status:** ⚠️ TODO - Document and test feature compatibility

## Performance Optimizations

### 1. Zero-Copy WAL Reading

Read WAL frames directly without copying:

```rust
use memmap2::MmapOptions;

let file = File::open(&wal_path)?;
let mmap = unsafe { MmapOptions::new().map(&file)? };
// Parse WAL frames directly from memory-mapped file
```

### 2. Parallel Compression

Use all CPU cores for compression:

```rust
use rayon::prelude::*;

let compressed_chunks: Vec<_> = chunks
    .par_iter()
    .map(|chunk| zstd::encode_all(chunk, level))
    .collect();
```

### 3. Batched Publishing

Combine multiple small payloads:

```rust
let mut batch = Vec::new();
let mut batch_size = 0;

while let Ok(payload) = rx.try_recv() {
    batch.push(payload);
    batch_size += payload.size();

    if batch_size > MAX_BATCH_SIZE {
        break;
    }
}

publisher.publish_batch(batch).await?;
```

## Testing Strategy

### Unit Tests

```rust
#[cfg(test)]
mod tests {
    #[test]
    fn test_wal_parsing() {
        let wal_data = include_bytes!("../fixtures/sample.wal");
        let frames = parse_wal_frames(wal_data).unwrap();
        assert_eq!(frames.len(), 42);
    }

    #[tokio::test]
    async fn test_batch_trigger() {
        let batcher = Batcher::new(config);
        // Test time-based and size-based triggers
    }
}
```

### Integration Tests

```rust
#[tokio::test]
async fn test_end_to_end() {
    // Start test SQLite database
    let db = setup_test_db().await;

    // Start sequencer
    let sequencer = Sequencer::new(config).start().await;

    // Perform SQL operations
    db.execute("INSERT INTO test VALUES (1, 'data')").await;

    // Verify published to mock DA layers
    let published = mock_da.get_published().await;
    assert_eq!(published.len(), 1);
}
```

### Benchmarks

```rust
use criterion::{criterion_group, criterion_main, Criterion};

fn bench_compression(c: &mut Criterion) {
    c.bench_function("zstd_1mb", |b| {
        let data = vec![0u8; 1_000_000];
        b.iter(|| zstd::encode_all(&data[..], 3))
    });
}
```

## Deployment

### Docker Image

```dockerfile
FROM rust:1.75 as builder
WORKDIR /app
COPY . .
RUN cargo build --release --features tee

FROM ubuntu:22.04
RUN apt-get update && apt-get install -y ca-certificates
COPY --from=builder /app/target/release/synddb-sequencer /usr/local/bin/
ENTRYPOINT ["synddb-sequencer"]
```

### Resource Requirements

- **CPU**: 2 cores minimum (4 recommended)
- **Memory**: 512MB minimum (2GB recommended)
- **Disk**: 10GB for caching
- **Network**: 100Mbps for DA publishing

### Monitoring

All metrics and logs are sent to GCP Cloud Logging for centralized monitoring:

**Structured Log Events:**
- `wal_frames_processed` - WAL frame count and processing latency
- `batch_published` - Batch sequence, size, DA layer, publish latency
- `da_publish_latency` - Per-DA layer publish times
- `compression_ratio` - Compression effectiveness metrics
- `attestation_refresh` - Attestation token refresh events
- `key_loaded` - Ethereum key loaded from Secret Manager

**Health Endpoint:**
- Port 8080: `/health` - Basic health check
- Port 8080: `/metrics` - JSON metrics endpoint for external monitoring

## Security Considerations

### 1. Read-Only Access

Sequencer only reads from SQLite, never writes:

```rust
let conn = Connection::open_with_flags(
    &db_path,
    OpenFlags::SQLITE_OPEN_READ_ONLY
)?;
```

### 2. Data Integrity

All published data includes checksums:

```rust
pub struct PublishPayload {
    data: Vec<u8>,
    checksum: Blake3Hash,
    sequence: u64,
    attestation: Option<TeeAttestation>,
}
```

### 3. TEE Isolation and Key Security

Ethereum signing keys are protected by multiple layers:

1. **Container Isolation**: Application container cannot access sequencer memory
2. **Secret Manager Binding**: Keys only accessible to container with matching digest
3. **Memory Encryption**: AMD SEV-SNP encrypts all VM memory
4. **No Key Export**: Keys never serialized outside Secret Manager

```rust
// Sequencer loads key from Secret Manager on startup
let key_manager = KeyManager::init().await?;

// Application has no access to Secret Manager or sequencer memory
// Keys remain in sidecar process memory only
```

### 4. Rate Limiting

Prevent resource exhaustion:

```rust
use governor::{Quota, RateLimiter};

let limiter = RateLimiter::direct(Quota::per_second(100));
limiter.until_ready().await;
```

## Migration Path

For existing applications:

1. **Ensure WAL mode**:

```sql
PRAGMA journal_mode=WAL;
```

2. **Deploy sequencer**:

```bash
docker run -v /app/data:/data syndicate/synddb-sequencer
```

3. **Verify publishing**:

```bash
curl http://localhost:9090/metrics | grep synddb_
```

No application code changes required - the sequencer is completely passive and transparent to the application.
