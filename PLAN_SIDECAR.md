# PLAN_SIDECAR.md - Lightweight SQLite Monitor and Publisher

## Overview

The synddb-sidecar is a zero-configuration Rust process that attaches to any SQLite database using the **SQLite Session Extension** to capture deterministic changesets. It publishes logical database changes (INSERT/UPDATE/DELETE operations) to multiple DA layers. This approach is far more robust than WAL parsing and ensures validators can deterministically re-derive state. It requires zero application code changes and works with SQLite databases from any programming language.

## Architecture

```
┌──────────────────────────────────────────────────────────────┐
│                  Application (Any Language)                  │
│         Python/JS/Go/Rust/Java → SQLite API → app.db        │
└──────────────────────────────────────────────────────────────┘
                                │
                    Database Transactions (via Session Extension)
                                ↓
┌──────────────────────────────────────────────────────────────┐
│                      synddb-sidecar                          │
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
synddb-sidecar/
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

-- This allows sidecar to detect and publish schema changes
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

When the application changes the database schema, the sidecar immediately creates and publishes a full database snapshot. This is simpler and more reliable than DDL replay, and leverages our existing snapshot infrastructure.

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

2. **Sidecar Detects Schema Change**
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

The sidecar leverages GCP Confidential Space for hardware-protected Ethereum key management.
The sidecar runs in a **separate container** from the application within the same Confidential Space VM,
providing strong isolation while maintaining filesystem access to the SQLite database.

### Security Model: Same-VM, Separate Containers

```
┌──────────────────────────────────────────────────────────────────┐
│              GCP Confidential Space VM (TEE)                     │
│  Hardware Root of Trust (AMD SEV-SNP / Intel TDX)               │
│                                                                   │
│  ┌────────────────────────┐  ┌────────────────────────┐        │
│  │   Application          │  │   synddb-sidecar       │        │
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
│  sidecar memory space where Ethereum keys are held.             │
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
2. **Read-Only SQLite Access**: Sidecar opens DB with `SQLITE_OPEN_READ_ONLY` flag
3. **Memory Encryption**: AMD SEV-SNP encrypts all VM memory including both containers
4. **No Shared Memory**: Containers communicate only via filesystem (SQLite DB file)
5. **Principle of Least Privilege**: Application has no credentials to access Secret Manager
6. **Attestation Binding**: Keys in Secret Manager are bound to sidecar container digest only

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
/// bound to the sidecar container digest via Workload Identity.
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

        let secret_name = "synddb-sidecar-signing-key".to_string();

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
        let audience = "https://synddb.io/sidecar";

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
        // IAM policy ensures only sidecar container with matching digest can access
        secret_client
            .create_secret(
                project_id,
                secret_name,
                secret_data,
                Some(vec![
                    ("synddb/environment", "confidential-space"),
                    ("synddb/component", "sidecar"),
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
COPY --from=builder /app/target/release/synddb-sidecar /usr/local/bin/

# Create non-root user
RUN useradd -m -u 1000 synddb && \
    chown -R synddb:synddb /usr/local/bin/synddb-sidecar

USER synddb

# Health check
HEALTHCHECK --interval=30s --timeout=3s \
    CMD curl -f http://localhost:9090/health || exit 1

# Entry point with attestation initialization
ENTRYPOINT ["/usr/local/bin/synddb-sidecar"]
CMD ["--attestation", "confidential-space", "--config", "/config/sidecar.yaml"]
```

### Deployment Configuration

```yaml
# confidential-space-deployment.yaml
apiVersion: compute.cnrm.cloud.google.com/v1beta1
kind: ComputeInstance
metadata:
  name: synddb-sidecar-tee
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
    - email: synddb-sidecar@${PROJECT_ID}.iam.gserviceaccount.com
      scopes:
        - https://www.googleapis.com/auth/cloud-platform

  metadata:
    items:
      - key: tee-container-log-redirect
        value: "true"
      - key: tee-image-reference
        value: "gcr.io/${PROJECT_ID}/synddb-sidecar:latest"
      - key: tee-restart-policy
        value: "Always"
      - key: tee-env-ATTESTATION_AUDIENCE
        value: "https://synddb.io/sidecar"
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
  name: synddb-sidecar
spec:
  displayName: SyndDB Sidecar Service Account
---
apiVersion: iam.cnrm.cloud.google.com/v1beta1
kind: IAMPolicyMember
metadata:
  name: synddb-sidecar-secretmanager
spec:
  memberFrom:
    serviceAccountRef:
      name: synddb-sidecar
  role: roles/secretmanager.secretAccessor
  resourceRef:
    kind: Project
---
apiVersion: iam.cnrm.cloud.google.com/v1beta1
kind: IAMPolicy
metadata:
  name: synddb-sidecar-secret-policy
spec:
  resourceRef:
    kind: Secret
    name: synddb-sidecar-signing-key
  policy:
    bindings:
      - role: roles/secretmanager.secretAccessor
        members:
          - serviceAccount:synddb-sidecar@${PROJECT_ID}.iam.gserviceaccount.com
        condition:
          title: Only from Confidential Space
          expression: |
            assertion.sub == 'synddb-sidecar@${PROJECT_ID}.iam.gserviceaccount.com' &&
            'image_digest' in assertion &&
            assertion.image_digest == '${EXPECTED_IMAGE_DIGEST}'
```

### Configuration File

```yaml
# config/sidecar-confidential.yaml
database:
  path: "/data/app.db"
  wal_mode: true

attestation:
  enabled: true
  provider: "gcp-confidential-space"

  # GCP Confidential Space settings
  gcp:
    project_id: "${PROJECT_ID}"
    secret_name: "synddb-sidecar-signing-key"
    attestation_audience: "https://synddb.io/sidecar"

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

    // Start sidecar
    let sidecar = Sidecar::new(config).start().await;

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
COPY --from=builder /app/target/release/synddb-sidecar /usr/local/bin/
ENTRYPOINT ["synddb-sidecar"]
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

Sidecar only reads from SQLite, never writes:

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

1. **Container Isolation**: Application container cannot access sidecar memory
2. **Secret Manager Binding**: Keys only accessible to container with matching digest
3. **Memory Encryption**: AMD SEV-SNP encrypts all VM memory
4. **No Key Export**: Keys never serialized outside Secret Manager

```rust
// Sidecar loads key from Secret Manager on startup
let key_manager = KeyManager::init().await?;

// Application has no access to Secret Manager or sidecar memory
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

2. **Deploy sidecar**:

```bash
docker run -v /app/data:/data syndicate/synddb-sidecar
```

3. **Verify publishing**:

```bash
curl http://localhost:9090/metrics | grep synddb_
```

No application code changes required - the sidecar is completely passive and transparent to the application.
