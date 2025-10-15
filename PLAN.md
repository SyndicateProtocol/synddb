# SyndDB Core Implementation Plan

## Important Note on Core vs Extensions
This document describes the **SyndDB Core** - the foundational infrastructure that powers all extensions. For information about:
- **Building extensions for SyndDB**: See [EXTENSIBILITY.md](EXTENSIBILITY.md)
- **Example extensions** (DEXs, tokens, gaming): See [EXTENSIBILITY.md](EXTENSIBILITY.md)
- **Extension development guide**: See [EXTENSIBILITY.md](EXTENSIBILITY.md)

The SyndDB Core is **extension-agnostic** - it provides the database execution, replication, and blockchain integration that all extensions leverage through well-defined interfaces.

## Executive Summary
SyndDB is a high-performance blockchain database that replaces traditional EVM execution with SQLite, enabling ultra-low latency database operations while maintaining decentralized validation. The system consists of a single sequencer node and multiple read replica nodes that anyone can run permissionlessly. Only a small subset of read replicas with TEE hardware become validators for settlement operations. This plan outlines a phased approach to build the complete system in **Rust**, starting with core architecture, focusing on SQLite performance, and progressively adding blockchain integration and validation capabilities.

## Architecture Overview

### Terminology Clarification
To avoid confusion, this document uses specific terms that emphasize local vs distributed execution:

**Local Execution (Sequencer Only):**
- **LocalWrite** - SQL statements executed locally in the sequencer's SQLite (INSERT, UPDATE, DELETE)
- **execute_local_write()** - Executes SQL immediately in the sequencer's local database
- **LocalWriteReceipt** - Instant confirmation of local execution (<1ms latency)

**Distributed Consensus (Blockchain):**
- **submit_writes_to_chain()** - Batches local writes and submits to blockchain for replication
- **ChainSubmitQueue** - Queue of local writes awaiting blockchain submission
- **ChainSubmitReceipt** - Confirmation that writes were submitted to blockchain

**State Replication Terms:**
- **DatabaseSnapshot** - Complete SQLite database backup at a specific version (familiar to all devs)
- **DatabaseDiff** - Incremental changes between versions (like git diff, familiar to all devs)
- **generate_diff()** / **apply_diff()** - Create/apply incremental changes
- **generate_snapshot()** / **apply_snapshot()** - Create/apply full database state

The flow is: Local writes execute instantly → batched periodically → submitted to chain for replication.

### System Components
```
┌─────────────────────────────────────────────────────────────────┐
│                         SyndDB System                           │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  ┌──────────────────┐       ┌──────────────────────────────┐  │
│  │  Sequencer (1)   │       │  Read Replicas (Anyone)      │  │
│  │                  │       │                              │  │
│  │  ┌─────────────┐ │       │  ┌──────────────────────┐   │  │
│  │  │   SQLite    │ │       │  │   SQLite Replicas    │   │  │
│  │  │   Engine    │ │       │  │   (Multiple Nodes)   │   │  │
│  │  └─────────────┘ │       │  └──────────────────────┘   │  │
│  │                  │       │                              │  │
│  │  ┌─────────────┐ │       │  ┌──────────────────────┐   │  │
│  │  │  Op Handler │ │       │  │  Database Sync &     │   │  │
│  │  │  & Triggers │ │       │  │  Query Engines       │   │  │
│  │  └─────────────┘ │       │  └──────────────────────┘   │  │
│  │                  │       │                              │  │
│  │  ┌─────────────┐ │       │  ┌──────────────────────┐   │  │
│  │  │ Diff/Snap   │ │       │  │  Subset: TEE         │   │  │
│  │  │  Generator  │ │       │  │  Validators Only     │   │  │
│  │  └─────────────┘ │       │  └──────────────────────┘   │  │
│  └──────────────────┘       └──────────────────────────────┘  │
│           │                            ▲                       │
│           │                            │                       │
│           ▼                            │                       │
│  ┌──────────────────────────────────────────────────────────┐ │
│  │         Syndicate Chain Smart Contracts                  │ │
│  │    (writeDiff, writeSnapshot, pointers, ordering)        │ │
│  └──────────────────────────────────────────────────────────┘ │
│                                                                │
│  ┌──────────────────────────────────────────────────────────┐ │
│  │         Off-chain Storage (IPFS/Arweave)                 │ │
│  └──────────────────────────────────────────────────────────┘ │
│                                                                │
└────────────────────────────────────────────────────────────────┘
```

## Technology Stack

### Core Language: Rust
All components will be implemented in Rust for:
- Memory safety without garbage collection
- Excellent performance characteristics
- Strong concurrency primitives
- Production-ready ecosystem

### Recommended Rust Libraries

#### Database & Storage
- **rusqlite** - SQLite bindings with full feature support ✅ (actively maintained)
- **sqlx** - Async SQL toolkit (alternative to rusqlite for async contexts) ✅ (v0.8+, actively maintained)
- **r2d2** - Database connection pooling ✅ (actively maintained)
- **r2d2_sqlite** - SQLite support for r2d2 ✅ (actively maintained)
- **sled** - Embedded database for metadata storage ✅ (actively maintained)

#### Networking & Async
- **tokio** - Async runtime for networking and I/O ✅ (v1.41+, very actively maintained)
- **axum** - Web framework for API endpoints ✅ (v0.7+, very actively maintained)
- **actix-web** - Alternative web framework ✅ (v4.9+, actively maintained)
- **jsonrpsee** - JSON-RPC server/client implementation ✅ (v0.24+, actively maintained)
- **tonic** - gRPC framework for inter-node communication ✅ (v0.12+, actively maintained)
- **quinn** - QUIC implementation for low-latency networking ✅ (v0.11+, actively maintained)

#### Blockchain Integration
- **alloy** - Modern, high-performance Ethereum library ✅ (v1.0+, very actively maintained, replaces ethers-rs)

#### Compression & Serialization
- **zstd** - Zstandard compression ✅ (actively maintained)
- **bincode** - Binary serialization ✅ (v2.0-rc, actively maintained)
- **serde** - Serialization framework ✅ (v1.0+, very actively maintained, 27M+ downloads/month)
- **prost** - Protocol Buffers ✅ (v0.13+, actively maintained)

#### Cryptography & Security
- **sp1-sdk** - Succinct SP1 Zero Knowledge Virtual Machine ✅ (v3.0+, actively maintained by Succinct)
- **lit-rust-sdk** - Lit Protocol SDK ✅ (v0.2+, recently updated)
- **ring** - Cryptographic operations ✅ (v0.17+, actively maintained)
- **sha3** - SHA-3 and Keccak hashing ✅ (v0.10+, actively maintained)
- **ed25519-consensus** - Ed25519 signatures ✅ (v2.1+, actively maintained)

#### Storage Backends
- **ipfs-api** - IPFS client ✅ (actively maintained, uses Hyper/Actix)
- **arweave-sdk-rs** - Arweave SDK ✅ (actively maintained)

#### Utilities
- **tracing** - Structured logging and diagnostics ✅ (v0.1+, very actively maintained)
- **metrics** - Performance metrics collection ✅ (actively maintained)
- **clap** - Command-line argument parsing ✅ (v4.5+, very actively maintained)
- **config** - Configuration management ✅ (v0.14+, actively maintained)
- **thiserror** - Error derive macro ✅ (v2.0+, very actively maintained, 27M+ downloads/month)
- **anyhow** - Flexible error handling ✅ (v1.0+, very actively maintained)

#### Important Notes:
- ✅ = Available and actively maintained
- All libraries have been verified as actively maintained with recent updates
- All listed versions are as of late 2024/early 2025

## Implementation Phases

## Phase 1: Architecture Skeleton & Core Infrastructure (Week 1-2)

### Goals
- Establish project structure and core abstractions
- Set up development environment with all necessary tooling
- Create interfaces for all major components

### Tasks

#### 1.1 Project Setup (Rust Workspace)
```
synddb/
├── Cargo.toml                 # Workspace configuration
├── crates/
│   ├── synddb-core/           # Core database abstractions
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── database.rs       # SQLite abstraction layer
│   │       ├── types.rs          # Core type definitions
│   │       ├── config.rs         # Configuration management
│   │       └── lib.rs
│   ├── synddb-sequencer/      # Sequencer implementation
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── main.rs           # Sequencer binary
│   │       ├── sequencer.rs      # Main sequencer orchestrator
│   │       ├── batcher.rs        # Operation batching logic
│   │       ├── compressor.rs     # Database diff/snapshot compression
│   │       └── publisher.rs      # Blockchain publishing
│   ├── synddb-replica/        # Read replica implementation
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── main.rs           # Replica binary
│   │       ├── replica.rs        # Main read replica orchestrator
│   │       ├── syncer.rs         # Database sync from blockchain
│   │       ├── reconstructor.rs  # Database reconstruction
│   │       ├── query.rs          # Query interface
│   │       └── validator.rs      # Optional: TEE validator
│   ├── synddb-contracts/      # Smart contract bindings
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── abi/              # Contract ABIs
│   │       ├── bindings.rs       # Generated bindings (alloy)
│   │       └── lib.rs
│   ├── synddb-storage/        # Storage backends
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── ipfs.rs           # IPFS integration
│   │       ├── arweave.rs        # Arweave integration
│   │       └── lib.rs
│   └── synddb-utils/          # Shared utilities
│       ├── Cargo.toml
│       └── src/
│           ├── compression.rs    # Compression utilities
│           ├── hashing.rs        # Hashing utilities
│           ├── metrics.rs        # Metrics collection
│           └── lib.rs
├── contracts/                 # Solidity contracts
│   ├── src/
│   └── test/
├── tests/                     # Integration tests
│   ├── integration/
│   └── benchmarks/
├── scripts/
│   ├── deploy.sh
│   └── migrate.sh
└── docs/
    ├── api/
    └── architecture/
```

#### 1.2 Core Traits and Types (Rust)
```rust
// synddb-core/src/database.rs
use async_trait::async_trait;
use rusqlite::{Connection, Transaction};
use serde::{Deserialize, Serialize};

#[async_trait]
pub trait SyndDatabase: Send + Sync {
    async fn begin_database_transaction(&self) -> Result<DatabaseTransaction>;
    async fn execute(&self, sql: &str, params: &[&dyn rusqlite::ToSql]) -> Result<ExecuteResult>;

    // SEQUENCER methods - Create replication data from local state
    // generate_snapshot: Creates a full database backup for replica bootstrapping
    // Think of this like `pg_dump` or `mysqldump` - a complete state capture
    async fn generate_snapshot(&self) -> Result<DatabaseSnapshot>;

    // generate_diff: Creates incremental changes since last version
    // Similar to git diff or database transaction logs - just the changes
    async fn generate_diff(&self, from_version: u64, to_version: u64) -> Result<DatabaseDiff>;

    // REPLICA methods - Apply replication data to reconstruct state
    // apply_snapshot: Restore from full backup (like `pg_restore`)
    // Replaces entire local database with the snapshot state
    async fn apply_snapshot(&self, snapshot: DatabaseSnapshot) -> Result<()>;

    // apply_diff: Apply incremental changes (like replaying a transaction log)
    // Updates local database by executing the changes in the diff
    async fn apply_diff(&self, diff: DatabaseDiff) -> Result<()>;
}

// synddb-sequencer/src/sequencer.rs
#[async_trait]
pub trait Sequencer: Send + Sync {
    async fn start(&mut self) -> Result<()>;
    async fn stop(&mut self) -> Result<()>;

    // EXECUTE_LOCAL_WRITE - Accepts SQL writes from clients for LOCAL execution only
    // This executes immediately in the sequencer's local SQLite (not replicated yet)
    // Returns instantly after local execution (<1ms latency)
    // Example: User places order, transfers tokens, or any SQL write operation
    // The "local" prefix makes it clear this is NOT distributed consensus
    async fn execute_local_write(&self, write: LocalWrite) -> Result<LocalWriteReceipt>;

    // SUBMIT_WRITES_TO_CHAIN - Periodically submits accumulated local writes to blockchain
    // This batches many local writes and creates either:
    // - A diff (incremental changes since last submission), OR
    // - A snapshot (complete database state for checkpointing)
    // Runs on a schedule (e.g., every 1 second or every 1000 local writes)
    // This enables read replicas to replicate the sequencer's state
    //
    // VERSION MANAGEMENT:
    // The Sequencer internally tracks:
    // - lastPublishedVersion: The last version successfully submitted to chain
    // - currentVersion: The current version after local writes
    // When creating diffs, it uses (from: lastPublishedVersion, to: currentVersion)
    // The BlockchainPublisher doesn't need to know about versions - they're embedded in the diff/snapshot
    async fn submit_writes_to_chain(&self) -> Result<ChainSubmitReceipt>;
}

// synddb-replica/src/replica.rs
#[async_trait]
pub trait ReadReplica: Send + Sync {
    async fn start(&mut self) -> Result<()>;
    async fn stop(&mut self) -> Result<()>;
    async fn sync_to_latest(&self) -> Result<()>;
    async fn query(&self, sql: &str, params: &[&dyn rusqlite::ToSql]) -> Result<QueryResult>;
    async fn subscribe_to_updates(&self, callback: UpdateCallback) -> Result<Subscription>;
}

// synddb-storage/src/lib.rs
#[async_trait]
pub trait StorageProvider: Send + Sync {
    async fn store(&self, data: &[u8]) -> Result<String>; // Returns CID/pointer
    async fn retrieve(&self, cid: &str) -> Result<Vec<u8>>;
}

// synddb-contracts/src/lib.rs
use alloy::primitives::B256;

// BlockchainPublisher is a THIN WRAPPER for blockchain interactions
// HYBRID APPROACH: Passes both global hash (for consistency) and type-specific hash
#[async_trait]
pub trait BlockchainPublisher: Send + Sync {
    // Diff methods pass global hash + diff-specific hash
    async fn submit_diff_to_chain(&self, diff: &[u8], prior_global: &[u8; 32], new_global: &[u8; 32], diff_hash: &[u8; 32]) -> Result<B256>;
    async fn submit_diff_pointer_to_chain(&self, cid: &str, prior_global: &[u8; 32], new_global: &[u8; 32], diff_hash: &[u8; 32]) -> Result<B256>;

    // Snapshot methods pass global hash + snapshot-specific hash
    async fn submit_snapshot_to_chain(&self, snapshot: &[u8], prior_global: &[u8; 32], new_global: &[u8; 32], snapshot_hash: &[u8; 32]) -> Result<B256>;
    async fn submit_snapshot_pointer_to_chain(&self, cid: &str, prior_global: &[u8; 32], new_global: &[u8; 32], snapshot_hash: &[u8; 32]) -> Result<B256>;
}
```

#### 1.3 Configuration System
```yaml
# config.yaml
synddb:
  role: sequencer  # or replica

  database:
    path: ./data/synddb.sqlite
    journal_mode: WAL           # Keep WAL for concurrent reads during writes
    synchronous: NORMAL         # NORMAL is optimal - FULL is overkill, OFF risks corruption
    cache_size: -2000000        # 2GB cache (was 64MB) - more RAM for hot data
    mmap_size: 274877906944     # 256GB mmap (was 30GB) - map entire DB to memory if possible

  sequencer:
    batch_size: 1000
    batch_timeout_ms: 100
    compression: zstd
    publish_interval_ms: 1000
    max_diff_size: 1048576  # 1MB
    blockchain_snapshot_interval: 10000  # Every 10k database operations

  replica:
    sync_interval_ms: 500
    cache_ttl_ms: 60000
    max_lag_blocks: 100

  chain:
    rpc_url: https://rpc.syndicate.io
    contract_address: "0x..."
    private_key: "${PRIVATE_KEY}"
    gas_limit: 3000000

  storage:
    provider: ipfs  # or arweave
    ipfs:
      gateway: https://ipfs.io
      api_endpoint: http://localhost:5001
    arweave:
      gateway: https://arweave.net
      wallet_path: ./arweave-wallet.json
```

## Phase 2: SQLite Database Engine & Performance Optimization (Week 3-5)

### Goals
- Implement high-performance SQLite wrapper optimized for blockchain use cases
- Build transaction handling with proper ACID guarantees
- Implement and benchmark performance optimizations
- Create database schema for common use cases

### Performance Philosophy
Since we have validators handling bridging/settlement and periodic blockchain commits:
- **Before blockchain commit**: Data must be durable to OS (NORMAL sync) but not necessarily to disk
- **After blockchain commit**: Data is replicated across read replicas, so single-node disk failure is acceptable
- **Trade-offs we accept**: Lower disk-level durability for 10-100x performance gains
- **What we DON'T compromise**: Data consistency and OS-level durability before state commitment

### Tasks

#### 2.1 SQLite Wrapper Implementation (Rust)
```rust
// synddb-core/src/database.rs
use rusqlite::{Connection, Transaction, OptionalExtension};
use r2d2::{Pool, PooledConnection};
use r2d2_sqlite::SqliteConnectionManager;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, debug};
use metrics::{counter, histogram};
use std::time::Instant;

pub struct SyndDatabase {
    pool: Pool<SqliteConnectionManager>,
    wal_manager: Arc<RwLock<WALManager>>,
    stats: Arc<RwLock<PerformanceStats>>,
    prepared_statements: Arc<RwLock<PreparedStatementCache>>,
}

impl SyndDatabase {
    pub fn new(config: DatabaseConfig) -> Result<Self> {
        let manager = SqliteConnectionManager::file(&config.path);
        let pool = Pool::builder()
            .max_size(config.pool_size)
            .build(manager)?;

        // Initialize with optimizations
        let conn = pool.get()?;
        Self::initialize_optimizations(&conn)?;

        Ok(Self {
            pool,
            wal_manager: Arc::new(RwLock::new(WALManager::new())),
            stats: Arc::new(RwLock::new(PerformanceStats::new())),
            prepared_statements: Arc::new(RwLock::new(PreparedStatementCache::new())),
        })
    }

    /// Called by execute_local_write() - executes SQL immediately in LOCAL database only
    /// This provides ultra-low latency (<1ms) since there's no distributed consensus
    /// The write is durable locally but not yet replicated to other nodes
    pub async fn execute_local_write(&self, write: LocalWrite) -> Result<LocalWriteReceipt> {
        let conn = self.pool.get()?;
        let start = Instant::now();

        // Execute SQL immediately in LOCAL SQLite (not distributed)
        conn.execute(&write.sql, &write.params)?;

        let receipt = LocalWriteReceipt {
            write_id: generate_write_id(),
            status: "committed_locally",
            latency: start.elapsed(),
            replication_eta: "~1s",  // Will be replicated in next chain submission
        };

        // Queue for later replication via submit_writes_to_chain()
        self.chain_submit_queue.enqueue(write).await?;

        Ok(receipt)
    }

    fn initialize_optimizations(conn: &Connection) -> Result<()> {
        // MAXIMUM PERFORMANCE CONFIGURATION
        // Optimized for write-heavy sequencer with state commitment guarantees

        // WAL mode for concurrent reads during writes
        conn.pragma_update(None, "journal_mode", "WAL")?;

        // NORMAL: Ensures durability to OS, but not to disk on every write
        // This is safe because we commit state to blockchain periodically
        conn.pragma_update(None, "synchronous", "NORMAL")?;

        // 2GB cache - keep hot data in RAM (negative = size in KB)
        conn.pragma_update(None, "cache_size", -2000000)?;

        // 256GB memory map - map entire DB file to virtual memory if possible
        conn.pragma_update(None, "mmap_size", 274877906944i64)?;

        // Keep temp tables/indices in memory
        conn.pragma_update(None, "temp_store", "MEMORY")?;

        // EXCLUSIVE mode - single sequencer doesn't need to coordinate
        conn.pragma_update(None, "locking_mode", "EXCLUSIVE")?;

        // 64KB pages (max size) - reduces B-tree depth for large datasets
        // Must be set before any tables are created
        conn.pragma_update(None, "page_size", 65536)?;

        // WAL optimizations for write performance
        conn.pragma_update(None, "wal_autocheckpoint", 10000)?;  // 10k pages before checkpoint
        conn.pragma_update(None, "wal_checkpoint", "PASSIVE")?;   // Non-blocking checkpoints

        // Optimize for SSDs - no need for sequential write optimization
        conn.pragma_update(None, "auto_vacuum", "INCREMENTAL")?;
        conn.pragma_update(None, "incremental_vacuum", 1000)?;    // Cleanup 1000 pages at a time

        // Memory optimizations
        conn.pragma_update(None, "threads", 0)?;                  // Single-threaded per connection

        info!("SQLite maximum performance optimizations applied");
        Ok(())
    }

    pub async fn execute_sql_batch(&self, operations: Vec<SqlOperation>) -> Result<BatchResult> {
        let start = Instant::now();
        let conn = self.pool.get()?;

        // Start a database transaction (atomic batch of SQL operations)
        let db_tx = conn.transaction()?;
        let mut results = Vec::new();

        for sql_op in operations.iter() {
            match self.execute_single(&db_tx, sql_op) {
                Ok(result) => results.push(result),
                Err(e) => {
                    db_tx.rollback()?;
                    return Err(e);
                }
            }
        }

        db_tx.commit()?;

        let duration = start.elapsed();
        histogram!("synddb.batch.duration", duration);
        counter!("synddb.batch.operations", operations.len() as u64);

        Ok(BatchResult {
            success: true,
            results,
            duration,
        })
    }
}
```

#### 2.2 Schema Extension Interface

Extensions define their schemas through the extension interface rather than being hardcoded in the Core. The SyndDB Core provides the infrastructure for executing and replicating these schemas.

```rust
// Extensions implement SchemaExtension trait
pub trait SchemaExtension: Send + Sync {
    fn schema_id(&self) -> &str;
    fn version(&self) -> u32;
    fn create_statements(&self) -> Vec<String>;
    fn migrate_statements(&self, from_version: u32) -> Result<Vec<String>>;
    fn index_statements(&self) -> Vec<String>;
    fn seed_statements(&self) -> Vec<String>;
}
```

For specific schema examples including order books, ERC-20 tokens, gaming leaderboards, and more, see [EXTENSIBILITY.md](EXTENSIBILITY.md). The SyndDB Core remains agnostic to extension-specific schemas.

#### 2.3 Performance Benchmarking Suite (Rust)
```rust
// tests/benchmarks/performance.rs
use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};
use synddb_core::SyndDatabase;
use std::time::Duration;
use hdrhistogram::Histogram;

pub struct PerformanceBenchmark {
    db: SyndDatabase,
    scenarios: Vec<Scenario>,
}

pub struct Scenario {
    name: String,
    setup: Box<dyn Fn(&SyndDatabase) -> Result<()>>,
    workload: Box<dyn Fn() -> Vec<SqlTransaction>>,
    target_tps: u64,
    target_latency_p99_ms: u64,
}

impl PerformanceBenchmark {
    pub fn new(db: SyndDatabase) -> Self {
        // Load scenarios from registered extensions
        let scenarios = Self::load_extension_scenarios();
        Self { db, scenarios }
    }

    /// Extensions provide their own benchmark scenarios
    pub fn load_extension_scenarios() -> Vec<Scenario> {
        vec![
            Scenario {
                name: "High-frequency writes".to_string(),
                setup: Box::new(|db| Self::setup_write_benchmark(db)),
                workload: Box::new(|| Self::generate_write_workload(10000)),
                target_tps: 50000,
                target_latency_p99_ms: 5,
            },
            Scenario {
                name: "Bulk inserts".to_string(),
                setup: Box::new(|db| Self::setup_bulk_benchmark(db)),
                workload: Box::new(|| Self::generate_bulk_inserts(10000)),
                target_tps: 100000,
                target_latency_p99_ms: 2,
            },
            Scenario {
                name: "Complex queries".to_string(),
                setup: Box::new(|db| Self::setup_query_benchmark(db)),
                workload: Box::new(|| Self::generate_complex_queries(100)),
                target_tps: 1000,
                target_latency_p99_ms: 50,
            },
        ]
    }

    pub async fn run_benchmarks(&self) -> Result<()> {
        for scenario in &self.scenarios {
            info!("Running benchmark: {}", scenario.name);

            (scenario.setup)(&self.db)?;
            let workload = (scenario.workload)();

            let results = self.execute_workload(workload).await?;
            self.analyze_results(&results, scenario);
        }

        Ok(())
    }

    fn analyze_results(&self, results: &BenchmarkResults, scenario: &Scenario) {
        let throughput = results.total_ops as f64 / results.duration.as_secs_f64();
        let p50 = results.latencies.value_at_percentile(50.0);
        let p99 = results.latencies.value_at_percentile(99.0);
        let p999 = results.latencies.value_at_percentile(99.9);

        let meets_targets = MeetsTargets {
            throughput: throughput >= scenario.target_tps as f64,
            latency: p99 <= scenario.target_latency_p99_ms * 1000, // Convert to microseconds
        };

        info!("Benchmark results for {}:", scenario.name);
        info!("  Throughput: {:.2} TPS (target: {})", throughput, scenario.target_tps);
        info!("  Latency P50: {}μs", p50);
        info!("  Latency P99: {}μs (target: {}ms)", p99, scenario.target_latency_p99_ms);
        info!("  Latency P99.9: {}μs", p999);
        info!("  Meets targets: {:?}", meets_targets);
    }
}

// Criterion benchmarks
pub fn bench_sqlite_operations(c: &mut Criterion) {
    let db = SyndDatabase::new(test_config()).unwrap();

    c.bench_function("single_insert", |b| {
        b.iter(|| {
            db.execute("INSERT INTO test VALUES (?)", &[&black_box(42)])
        });
    });

    c.bench_function("batch_insert_100", |b| {
        b.iter(|| {
            let operations = generate_test_operations(100);
            db.execute_sql_batch(black_box(operations))
        });
    });
}

criterion_group!(benches, bench_sqlite_operations);
criterion_main!(benches);
```

#### 2.4 Performance Optimization Techniques

##### Prepared Statements Cache (Rust)
```rust
// synddb-core/src/prepared_statements.rs
use rusqlite::{Connection, Statement, Result};
use std::collections::HashMap;
use parking_lot::RwLock;

pub struct PreparedStatementCache {
    statements: RwLock<HashMap<String, String>>,
}

impl PreparedStatementCache {
    pub fn new() -> Self {
        Self {
            statements: RwLock::new(HashMap::new()),
        }
    }

    pub fn register(&self, key: &str, sql: &str) {
        let mut cache = self.statements.write();
        cache.insert(key.to_string(), sql.to_string());
    }

    pub fn get(&self, key: &str) -> Option<String> {
        let cache = self.statements.read();
        cache.get(key).cloned()
    }

    // Initialize common prepared statements
    pub fn initialize_common(&self) {
        self.register(
            "insert_order",
            r#"
            INSERT INTO orders (order_id, account_id, side, price, quantity,
                              remaining_quantity, status, created_at, updated_at, nonce)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            "#,
        );

        self.register(
            "update_order_status",
            r#"
            UPDATE orders
            SET status = ?1, remaining_quantity = ?2, updated_at = ?3
            WHERE order_id = ?4
            "#,
        );

        self.register(
            "transfer_balance",
            r#"
            UPDATE balances
            SET balance = balance + ?1
            WHERE account_id = ?2 AND token_address = ?3
            "#,
        );
    }
}
```

##### Connection Pooling (Rust)
```rust
// Using r2d2 for connection pooling (already shown in database.rs)
// Additional configuration example:

use r2d2::{Pool, PooledConnection, CustomizeConnection};
use r2d2_sqlite::SqliteConnectionManager;
use std::time::Duration;

#[derive(Debug)]
struct ConnectionCustomizer;

impl CustomizeConnection<rusqlite::Connection, rusqlite::Error> for ConnectionCustomizer {
    fn on_acquire(&self, conn: &mut rusqlite::Connection) -> Result<(), rusqlite::Error> {
        // Set connection-specific pragmas
        conn.pragma_update(None, "busy_timeout", 5000)?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        Ok(())
    }
}

pub fn create_connection_pool(path: &str, size: u32) -> Result<Pool<SqliteConnectionManager>> {
    let manager = SqliteConnectionManager::file(path);

    Pool::builder()
        .max_size(size)
        .min_idle(Some(size / 4))
        .connection_timeout(Duration::from_secs(30))
        .idle_timeout(Some(Duration::from_secs(600)))
        .connection_customizer(Box::new(ConnectionCustomizer))
        .build(manager)
}
```

## Phase 3: Local Write Type System & SQLite Triggers (Week 6-7)

### Goals
- Build flexible local write type system using SQLite triggers
- Implement validation and business logic at database level
- Create programmable hooks for custom write types
- Build write serialization and deserialization

### Tasks

#### 3.1 Extension Registry System (Rust)
```rust
// synddb-core/src/extensions.rs
use std::collections::HashMap;
use async_trait::async_trait;

/// Central registry for all extensions
pub struct ExtensionRegistry {
    schemas: HashMap<String, Box<dyn SchemaExtension>>,
    writes: HashMap<String, Box<dyn LocalWriteExtension>>,
    triggers: HashMap<String, Box<dyn TriggerExtension>>,
    bridges: HashMap<String, Box<dyn BridgeExtension>>,
    queries: HashMap<String, Box<dyn QueryExtension>>,
}

impl ExtensionRegistry {
    pub fn new() -> Self {
        Self {
            schemas: HashMap::new(),
            writes: HashMap::new(),
            triggers: HashMap::new(),
            bridges: HashMap::new(),
            queries: HashMap::new(),
        }
    }

    pub fn register_schema(&mut self, extension: Box<dyn SchemaExtension>) -> Result<()> {
        let id = extension.schema_id().to_string();
        if self.schemas.contains_key(&id) {
            return Err(anyhow!("Schema {} already registered", id));
        }
        self.schemas.insert(id, extension);
        Ok(())
    }

    pub fn register_write(&mut self, extension: Box<dyn LocalWriteExtension>) -> Result<()> {
        let write_type = extension.write_type().to_string();
        if self.writes.contains_key(&write_type) {
            return Err(anyhow!("Write type {} already registered", write_type));
        }
        self.writes.insert(write_type, extension);
        Ok(())
    }

    pub fn register_trigger(&mut self, extension: Box<dyn TriggerExtension>) -> Result<()> {
        let id = extension.trigger_id().to_string();
        if self.triggers.contains_key(&id) {
            return Err(anyhow!("Trigger {} already registered", id));
        }
        self.triggers.insert(id, extension);
        Ok(())
    }

    pub fn register_bridge(&mut self, extension: Box<dyn BridgeExtension>) -> Result<()> {
        let id = extension.bridge_id().to_string();
        if self.bridges.contains_key(&id) {
            return Err(anyhow!("Bridge {} already registered", id));
        }
        self.bridges.insert(id, extension);
        Ok(())
    }

    /// Initialize all registered extensions
    pub async fn initialize(&self, database: &SyndDatabase) -> Result<()> {
        // Install schemas
        for schema in self.schemas.values() {
            for statement in schema.create_statements() {
                database.execute(&statement, &[]).await?;
            }
            for statement in schema.index_statements() {
                database.execute(&statement, &[]).await?;
            }
        }

        // Install triggers
        for trigger in self.triggers.values() {
            let sql = format!(
                "CREATE TRIGGER {} {} {} ON {} BEGIN {} END",
                trigger.trigger_id(),
                match trigger.trigger_event() {
                    TriggerEvent::BeforeInsert => "BEFORE INSERT",
                    TriggerEvent::AfterInsert => "AFTER INSERT",
                    TriggerEvent::BeforeUpdate => "BEFORE UPDATE",
                    TriggerEvent::AfterUpdate => "AFTER UPDATE",
                    TriggerEvent::BeforeDelete => "BEFORE DELETE",
                    TriggerEvent::AfterDelete => "AFTER DELETE",
                },
                trigger.table_name(),
                trigger.trigger_sql()
            );
            database.execute(&sql, &[]).await?;
        }

        Ok(())
    }
}
```

#### 3.2 SQLite Trigger Interface

Extensions register SQLite triggers through the extension interface to implement business logic at the database level. The SyndDB Core manages trigger installation and execution order.

```rust
// Extensions implement TriggerExtension trait
pub trait TriggerExtension: Send + Sync {
    fn trigger_id(&self) -> &str;
    fn table_name(&self) -> &str;
    fn trigger_event(&self) -> TriggerEvent;
    fn trigger_sql(&self) -> String;
    fn dependencies(&self) -> Vec<String>;
}

pub enum TriggerEvent {
    BeforeInsert, AfterInsert,
    BeforeUpdate, AfterUpdate,
    BeforeDelete, AfterDelete,
}

// SyndDB Core handles trigger registration
impl LocalWriteRegistry {
    fn install_triggers(&self, definition: &dyn LocalWriteDefinition) -> Result<()> {
        let trigger_sql = self.generate_trigger_sql(definition)?;
        let conn = self.get_connection()?;
        conn.execute_batch(&trigger_sql)?;
        Ok(())
    }
}
```

For specific trigger examples including order matching, balance validation, liquidation monitoring, and more complex business logic implementations, see [EXTENSIBILITY.md](EXTENSIBILITY.md). The SyndDB Core provides the trigger execution infrastructure while extensions define their specific logic.

#### 3.3 Local Write Framework (Rust)
```rust
// synddb-core/src/writes.rs

// Core LocalWrite structure that all extensions use
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalWrite {
    pub write_type: String,
    pub request: serde_json::Value,
    pub timestamp: u64,
    pub nonce: u64,
}

// Extensions provide their own write implementations
pub trait LocalWriteExtension: Send + Sync {
    fn write_type(&self) -> &str;
    fn schema(&self) -> &serde_json::Value;
    fn validate(&self, request: &serde_json::Value) -> Result<()>;
    fn to_sql(&self, request: &serde_json::Value) -> Result<Vec<SqlStatement>>;

    // Optional hooks for pre/post processing
    fn pre_execute(&self, request: &serde_json::Value) -> Result<()> {
        Ok(())
    }

    fn post_execute(&self, request: &serde_json::Value, result: &ExecuteResult) -> Result<()> {
        Ok(())
    }
}

// Core engine processes writes generically
impl Sequencer {
    pub async fn execute_local_write(&self, write: LocalWrite) -> Result<LocalWriteReceipt> {
        // Find the registered extension for this write type
        let extension = self.registry
            .get_write_extension(&write.write_type)
            .ok_or_else(|| anyhow!("Unknown write type: {}", write.write_type))?;

        // Validate using extension
        extension.validate(&write.request)?;

        // Pre-execution hook
        extension.pre_execute(&write.request)?;

        // Convert to SQL using extension
        let sql_statements = extension.to_sql(&write.request)?;

        // Execute in local SQLite
        let result = self.database.execute_batch(sql_statements).await?;

        // Post-execution hook
        extension.post_execute(&write.request, &result)?;

        // Queue for blockchain submission
        self.chain_submit_queue.enqueue(write).await?;

        Ok(LocalWriteReceipt {
            write_id: generate_write_id(),
            status: "committed_locally",
            latency: result.duration,
            replication_eta: "~1s",
        })
    }
}
```

## Phase 4: Blockchain Integration & Database Publishing (Week 8-9)

### Goals
- Implement smart contracts for database state publication
- Build database diff and database snapshot generation system
- Implement compression and chunking for large states
- Create off-chain storage integration (IPFS/Arweave)

### Local Write Flow Overview
```
LOCAL EXECUTION (Sequencer Only):
Client Request → execute_local_write() → Commit to Local SQLite → Return Receipt
  (e.g., place order)    (SQL write)        (local only)        (instant, <1ms)
                                              ↓
                                    [Queue in ChainSubmitQueue]
                                              ↓
DISTRIBUTED REPLICATION (via Blockchain):
              Timer/Threshold → submit_writes_to_chain() → generate_diff() or generate_snapshot()
                                              ↓
                                    Create Blockchain Transaction
                                              ↓
                                    Submit to Chain for Consensus
                                              ↓
                                    Read Replicas apply_diff() / apply_snapshot()
                                              ↓
                                    All Nodes Have Consistent State
```

### Tasks

#### 4.1 Smart Contract Implementation
```solidity
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract SyndDB {
    // HYBRID HASH CHAIN APPROACH:
    // - Global chain (lastHash): Ensures total ordering and sequencer consistency
    // - Per-type chains: Enable flexible traversal for read replicas

    // Primary chain - every operation MUST update this
    bytes32 public lastHash;  // Global state hash (ensures sequencer is always up-to-date)

    // Secondary chains - for type-specific traversal by read replicas
    bytes32 public lastDiffHash;      // Last diff hash (for incremental sync)
    bytes32 public lastSnapshotHash;  // Last snapshot hash (for bootstrap)

    address public sequencer;
    uint256 public currentVersion;

    // Events include both global and type-specific hashes
    event DiffPublished(
        uint256 indexed fromVersion,
        uint256 indexed toVersion,
        bytes32 diffHash,
        bytes32 globalHash,
        bytes32 priorDiffHash
    );
    event SnapshotPublished(
        uint256 indexed version,
        bytes32 snapshotHash,
        bytes32 globalHash,
        bytes32 priorSnapshotHash
    );

    modifier onlySequencer() {
        require(msg.sender == sequencer, "Only sequencer can publish");
        _;
    }

    // Publish state diff directly to chain (for small state diffs)
    function publishDiff(
        bytes32 priorGlobalHash,  // Must match lastHash (ensures global consistency)
        bytes32 newGlobalHash,    // New global hash after this operation
        bytes32 diffHash,         // Hash of this specific diff
        uint256 diffIndex,
        bytes calldata diffData
    ) external onlySequencer {
        require(priorGlobalHash == lastHash, "Global hash mismatch");

        // Update both chains
        lastHash = newGlobalHash;
        bytes32 priorDiffHash = lastDiffHash;
        lastDiffHash = diffHash;

        // Store diff chunk - stub
    }

    // Publish state diff pointer (for large state diffs stored off-chain)
    function publishDiffPointer(
        bytes32 priorGlobalHash,  // Must match lastHash (ensures global consistency)
        bytes32 newGlobalHash,    // New global hash after this operation
        bytes32 diffHash,         // Hash of this specific diff
        uint256 fromVersion,
        uint256 toVersion,
        string calldata storagePointer
    ) external onlySequencer {
        require(priorGlobalHash == lastHash, "Global hash mismatch");
        require(toVersion > fromVersion, "Invalid version range");

        // Store prior hashes for event
        bytes32 priorDiffHash = lastDiffHash;

        // Update all state
        lastHash = newGlobalHash;
        lastDiffHash = diffHash;
        currentVersion = toVersion;

        emit DiffPublished(fromVersion, toVersion, diffHash, newGlobalHash, priorDiffHash);
    }

    // Publish state snapshot directly to chain
    function publishSnapshot(
        bytes32 priorGlobalHash,  // Must match lastHash (ensures global consistency)
        bytes32 newGlobalHash,    // New global hash after this operation
        bytes32 snapshotHash,     // Hash of this specific snapshot
        uint256 snapshotIndex,
        bytes calldata snapshotData
    ) external onlySequencer {
        require(priorGlobalHash == lastHash, "Global hash mismatch");

        // Update both chains
        lastHash = newGlobalHash;
        bytes32 priorSnapshotHash = lastSnapshotHash;
        lastSnapshotHash = snapshotHash;

        // Store snapshot chunk - stub
    }

    // Publish state snapshot pointer
    function publishSnapshotPointer(
        bytes32 priorGlobalHash,  // Must match lastHash (ensures global consistency)
        bytes32 newGlobalHash,    // New global hash after this operation
        bytes32 snapshotHash,     // Hash of this specific snapshot
        uint256 version,
        string calldata storagePointer
    ) external onlySequencer {
        require(priorGlobalHash == lastHash, "Global hash mismatch");

        // Store prior hashes for event
        bytes32 priorSnapshotHash = lastSnapshotHash;

        // Update state
        lastHash = newGlobalHash;
        lastSnapshotHash = snapshotHash;

        emit SnapshotPublished(version, snapshotHash, newGlobalHash, priorSnapshotHash);
    }

    // Helper view functions for read replicas
    function getLatestDiff() external view returns (bytes32) {
        return lastDiffHash;
    }

    function getLatestSnapshot() external view returns (bytes32) {
        return lastSnapshotHash;
    }

    function getGlobalState() external view returns (bytes32 globalHash, bytes32 diffHash, bytes32 snapshotHash, uint256 version) {
        return (lastHash, lastDiffHash, lastSnapshotHash, currentVersion);
    }
}
```

#### 4.2 State Diff Generation System (Rust)
```rust
// synddb-sequencer/src/diff_generator.rs
use serde::{Serialize, Deserialize};
use sha3::{Digest, Sha3_256};
use zstd;

/// DatabaseDiffGenerator is used by the SEQUENCER to create database diffs
/// These diffs contain SQL statements that represent changes between database versions
/// The generated diffs are submitted to the blockchain for read replicas to consume
pub struct DatabaseDiffGenerator {
    last_published_version: u64,
    change_log: ChangeLog,
}

impl DatabaseDiffGenerator {
    /// Generates a database diff containing all SQL changes between two versions
    /// This is called by the sequencer when submitting database updates to blockchain
    /// The output will be compressed and either stored on-chain or in IPFS/Arweave
    pub async fn generate_database_diff(&self, from_version: u64, to_version: u64) -> Result<DatabaseDiff> {
        // Get all changes between versions
        let changes = self.change_log.get_changes(from_version, to_version).await?;

        // Group changes by table
        let grouped_changes = self.group_by_table(changes);

        // Generate SQL statements to replay changes
        let mut sql_statements = Vec::new();
        for (table, table_changes) in grouped_changes {
            sql_statements.extend(self.generate_table_diff(&table, table_changes)?);
        }

        // Calculate checksum
        let checksum = self.calculate_checksum(&sql_statements);
        let timestamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)?
            .as_secs();

        // Create database diff object with versions embedded
        // The sequencer will track the hash chain separately
        let diff = DatabaseDiff {
            from_version,     // Version currently on chain
            to_version,       // Version after applying this diff
            statements: sql_statements.clone(),
            checksum,
            timestamp,
            compressed: Vec::new(),
            compressed_size: 0,
            compression_ratio: 0.0,
        };

        // Compress diff
        let json = serde_json::to_vec(&diff)?;
        let compressed = zstd::encode_all(&json[..], 3)?;
        let compression_ratio = compressed.len() as f64 / json.len() as f64;

        Ok(DatabaseDiff {
            compressed,
            compressed_size: compressed.len(),
            compression_ratio,
            ..diff
        })
    }

    fn generate_table_diff(&self, table: &str, changes: Vec<Change>) -> Result<Vec<String>> {
        let mut statements = Vec::new();

        for change in changes {
            let statement = match change.operation {
                Operation::Insert => self.generate_insert(table, &change.data)?,
                Operation::Update => self.generate_update(table, &change.data, &change.old_data)?,
                Operation::Delete => self.generate_delete(table, &change.data)?,
            };
            statements.push(statement);
        }

        Ok(statements)
    }

    fn calculate_checksum(&self, statements: &[String]) -> String {
        let mut hasher = Sha3_256::new();
        for statement in statements {
            hasher.update(statement.as_bytes());
        }
        format!("{:x}", hasher.finalize())
    }
}
```

#### 4.3 State Snapshot Generation
```typescript
class SnapshotGenerator {
  async generateSnapshot(): Promise<StateSnapshot> {
    const version = await this.getCurrentVersion();

    // Use SQLite backup API for consistent snapshot
    const backupPath = `/tmp/synddb-snapshot-${version}.db`;
    await this.db.backup(backupPath);

    // Read snapshot file
    const snapshotData = await fs.readFile(backupPath);

    // Generate merkle tree of database pages
    const merkleRoot = await this.generateMerkleRoot(snapshotData);

    // Compress snapshot
    const compressed = await zstd.compress(snapshotData, 6);

    const snapshot: StateSnapshot = {
      version,
      merkleRoot,
      size: snapshotData.length,
      compressedSize: compressed.length,
      timestamp: Date.now(),
      data: compressed
    };

    // Clean up temp file
    await fs.unlink(backupPath);

    return snapshot;
  }

  private async generateMerkleRoot(data: Buffer): Promise<string> {
    const pageSize = 4096; // SQLite page size
    const pageCount = Math.ceil(data.length / pageSize);

    const leaves: string[] = [];
    for (let i = 0; i < pageCount; i++) {
      const start = i * pageSize;
      const end = Math.min(start + pageSize, data.length);
      const page = data.slice(start, end);
      leaves.push(hash(page));
    }

    return this.buildMerkleTree(leaves);
  }
}
```

#### 4.4 IPFS/Arweave Integration
```typescript
class IPFSStorage implements IOffchainStorageProvider {
  private ipfs: IPFS;

  async storeToIPFS(data: Buffer): Promise<string> {
    // Add to IPFS with chunking for large files
    const options = {
      pin: true,
      chunker: 'rabin-avg-65536'
    };

    const result = await this.ipfs.add(data, options);
    return result.cid.toString();
  }

  async retrieveFromIPFS(cid: string): Promise<Buffer> {
    const chunks: Buffer[] = [];

    for await (const chunk of this.ipfs.cat(cid)) {
      chunks.push(chunk);
    }

    return Buffer.concat(chunks);
  }
}

class ArweaveStorage implements IOffchainStorageProvider {
  private arweave: Arweave;

  async storeToArweave(data: Buffer): Promise<string> {
    const transaction = await this.arweave.createTransaction({
      data: data.toString('base64'),
      tags: [
        { name: 'Content-Type', value: 'application/octet-stream' },
        { name: 'App-Name', value: 'SyndDB' },
        { name: 'Version', value: '1.0.0' }
      ]
    });

    await this.arweave.transactions.sign(transaction);
    await this.arweave.transactions.post(transaction);

    return transaction.id;
  }

  async retrieveFromArweave(id: string): Promise<Buffer> {
    const data = await this.arweave.transactions.getData(id, { decode: true });
    return Buffer.from(data);
  }
}
```

#### 4.5 State Publisher
```typescript
/**
 * BlockchainSubmitter handles the SUBMIT_WRITES_TO_CHAIN functionality of the Sequencer
 * It runs periodically to batch and submit accumulated local writes to the blockchain
 * This is separate from execute_local_write which handles individual client requests
 *
 * HYBRID HASH TRACKING:
 * - lastGlobalHash: Ensures sequencer has seen all prior operations (diffs AND snapshots)
 * - lastDiffHash: Tracks the diff-specific chain for incremental updates
 * - lastSnapshotHash: Tracks the snapshot-specific chain for bootstrapping
 *
 * Benefits:
 * 1. Sequencer must acknowledge ALL operations via global hash
 * 2. Read replicas can follow just diffs for incremental sync
 * 3. New replicas can bootstrap from snapshots without processing all diffs
 * 4. Replicas can verify they're on the correct chain via global hash
 */
class BlockchainSubmitter {
  private queue: ChainSubmitQueue;  // Queue of local writes submitted via execute_local_write
  private storage: IOffchainStorageProvider;
  private blockchainClient: IBlockchainPublisher;  // Thin wrapper for chain interaction
  private lastPublishedVersion: u64;  // VERSION TRACKING: Last version on chain
  private lastGlobalHash: bytes32;    // HASH TRACKING: Global chain
  private lastDiffHash: bytes32;      // HASH TRACKING: Diff-specific chain
  private lastSnapshotHash: bytes32;  // HASH TRACKING: Snapshot-specific chain

  /**
   * Called periodically (e.g., every second) to submit accumulated local writes to blockchain
   * This is the implementation of Sequencer.submit_writes_to_chain()
   */
  async submitWritesToChain() {
    // Get batch of local writes that were executed via execute_local_write
    // These have already been executed locally in the sequencer's SQLite database
    const batch = await this.queue.getBatch();

    if (batch.writes.length === 0) {
      return;  // Nothing new to publish
    }

    // Generate database diff from all local writes since last submission
    // This calls SyndDatabase.generate_diff() internally
    // IMPORTANT: The diff contains both from_version and to_version internally
    const diff = await this.diffGenerator.generateDiff(
      this.lastPublishedVersion,  // FROM: Where chain currently is
      batch.version                // TO: Where sequencer local DB is now
    );

    // Decide whether to publish directly onchain or via offchain storage
    const publishStrategy = this.determineBlockchainStrategy(diff);

    // BlockchainPublisher receives the diff with versions AND state hashes embedded
    // It passes these through to the smart contract for chain validation
    if (publishStrategy === 'direct') {
      await this.publishDirectlyOnchain(diff);
    } else {
      await this.publishViaOffchainStorage(diff);
    }

    // Update last published version only after successful submission
    this.lastPublishedVersion = batch.version;

    // Check if snapshot is needed
    if (this.shouldCreateSnapshot(batch.version)) {
      await this.publishSnapshotToBlockchain(batch.version);
    }
  }

  private determineBlockchainStrategy(diff: DatabaseDiff): 'direct' | 'offchain' {
    const MAX_ONCHAIN_SIZE = 100 * 1024; // 100KB
    return diff.compressedSize < MAX_ONCHAIN_SIZE ? 'direct' : 'offchain';
  }

  private async publishDirectlyOnchain(diff: DatabaseDiff) {
    const newGlobalHash = this.computeGlobalHash(diff);
    const diffHash = this.computeDiffHash(diff);

    await this.blockchainClient.submitDiffToChain(
      diff.compressed,
      this.lastGlobalHash,     // Must match global lastHash on chain
      newGlobalHash,           // New global hash
      diffHash                 // Hash of this specific diff
    );

    this.lastGlobalHash = newGlobalHash;
    this.lastDiffHash = diffHash;
  }

  private async publishViaOffchainStorage(diff: DatabaseDiff) {
    const cid = await this.storage.storeToIPFS(diff.compressed);
    const newGlobalHash = this.computeGlobalHash(diff);
    const diffHash = this.computeDiffHash(diff);

    await this.blockchainClient.submitDiffPointerToChain(
      cid,
      this.lastGlobalHash,     // Must match global lastHash on chain
      newGlobalHash,           // New global hash
      diffHash                 // Hash of this specific diff
    );

    this.lastGlobalHash = newGlobalHash;
    this.lastDiffHash = diffHash;
  }
}
```

## Phase 5: Read Replica Implementation (Week 10-11)

### Goals
- Build read replica nodes that sync from blockchain (anyone can run a replica)
- Implement state derivation from state diffs and state snapshots
- Create query interface for read replicas
- Build monitoring and alerting system for replica health
- Enable permissionless replica deployment - no special authorization needed

**Important Notes:**
- **Anyone can run a read replica** - replicas are permissionless and open to all
- **Read replicas provide data access and queries** - most replicas will only serve this function
- **Validators are a subset of read replicas** - only some replicas with TEE capabilities become validators (Phase 6)
- **No settlement authority for regular replicas** - only TEE validators can process withdrawals

### Tasks

#### 5.1 Read Replica Node Architecture
```typescript
class ReadReplicaNode {
  private db: SyndDatabase;
  private syncer: DatabaseSyncer;
  private queryEngine: QueryEngine;
  private monitor: ReplicaMonitor;

  async start() {
    // Initialize from latest snapshot or genesis
    await this.initialize();

    // Start sync process
    this.syncer.start();

    // Start query server
    await this.queryEngine.listen();

    // Start monitoring
    this.monitor.start();
  }

  private async initialize() {
    const latestSnapshot = await this.findLatestSnapshot();

    if (latestSnapshot) {
      await this.bootstrapFromSnapshot(latestSnapshot);
    } else {
      await this.bootstrapFromGenesis();
    }
  }

  private async bootstrapFromSnapshot(snapshot: SnapshotCommitment) {
    // Retrieve snapshot from storage
    const data = await this.storage.retrieve(snapshot.storagePointer);

    // Decompress
    const decompressed = await zstd.decompress(data);

    // Verify integrity
    const calculatedHash = hash(decompressed);
    if (calculatedHash !== snapshot.snapshotHash) {
      throw new Error('Snapshot integrity check failed');
    }

    // Restore database
    await this.db.restoreFromSnapshot(decompressed);

    // Set current version
    this.currentVersion = snapshot.version;
  }
}
```

#### 5.2 State Synchronization
```typescript
/**
 * DatabaseSyncer is used by READ REPLICAS to fetch and apply database updates
 * It retrieves snapshots/diffs submitted by the sequencer and applies them to the local database
 * This is the counterpart to the sequencer's generate_database_diff/generate_database_snapshot functions
 */
class DatabaseSyncer {
  private currentVersion: number = 0;
  private targetVersion: number = 0;
  private isSyncing: boolean = false;

  async start() {
    // Subscribe to blockchain events
    this.subscribeToChainEvents();

    // Start sync loop
    this.startSyncLoop();
  }

  private async startSyncLoop() {
    while (true) {
      try {
        await this.sync();
        await sleep(this.config.syncIntervalMs);
      } catch (error) {
        console.error('Sync error:', error);
        await this.handleSyncError(error);
      }
    }
  }

  private async sync() {
    // Get latest version from chain
    this.targetVersion = await this.chain.getCurrentVersion();

    if (this.targetVersion <= this.currentVersion) {
      return; // Already up to date
    }

    this.isSyncing = true;

    // Find diffs to apply
    const diffs = await this.findDiffsToApply(
      this.currentVersion,
      this.targetVersion
    );

    // Apply diffs sequentially
    for (const diff of diffs) {
      await this.applyDiff(diff);
    }

    this.currentVersion = this.targetVersion;
    this.isSyncing = false;
  }

  /**
   * Applies a database diff to the local database (calls SyndDatabase.apply_database_diff)
   * This reconstructs the sequencer's database changes by executing the SQL statements
   * contained in the diff that was generated by the sequencer's generate_database_diff method
   */
  private async applyDatabaseDiff(diffCommitment: DiffCommitment) {
    // Retrieve diff data
    let diffData: Buffer;

    if (diffCommitment.storagePointer) {
      // Retrieve from IPFS/Arweave
      diffData = await this.storage.retrieve(diffCommitment.storagePointer);
    } else {
      // Retrieve from chain
      diffData = await this.retrieveFromChain(diffCommitment);
    }

    // Decompress
    const decompressed = await zstd.decompress(diffData);
    const diff = JSON.parse(decompressed.toString()) as DatabaseDiff;

    // Verify integrity
    const calculatedHash = hash(decompressed);
    if (calculatedHash !== diffCommitment.diffHash) {
      throw new Error('Diff integrity check failed');
    }

    // Apply SQL statements - this calls the SyndDatabase.apply_database_diff trait method
    // which executes the SQL statements to update the replica's database
    await this.db.applyDatabaseDiff(diff);
  }
}
```

#### 5.3 Query Interface
```typescript
class QueryEngine {
  private server: FastifyInstance;
  private cache: QueryCache;

  async listen() {
    this.server = fastify();

    // Query endpoint
    this.server.post('/query', async (request, reply) => {
      const { sql, params } = request.body;

      // Validate query (read-only)
      if (!this.isReadOnlyQuery(sql)) {
        return reply.code(400).send({
          error: 'Only read queries are allowed'
        });
      }

      // Check cache
      const cacheKey = this.getCacheKey(sql, params);
      const cached = await this.cache.get(cacheKey);
      if (cached) {
        return cached;
      }

      // Execute query
      const result = await this.db.query(sql, params);

      // Cache result
      await this.cache.set(cacheKey, result, this.config.cacheTtl);

      return result;
    });

    // Health check
    this.server.get('/health', async () => {
      return {
        status: 'healthy',
        version: this.currentVersion,
        targetVersion: this.targetVersion,
        isSyncing: this.isSyncing,
        lag: this.targetVersion - this.currentVersion
      };
    });

    await this.server.listen({ port: 3000 });
  }

  private isReadOnlyQuery(sql: string): boolean {
    const normalized = sql.trim().toUpperCase();
    return normalized.startsWith('SELECT') ||
           normalized.startsWith('WITH');
  }
}
```

#### 5.4 Monitoring and Alerting
```typescript
class ReplicaMonitor {
  private metrics: MetricsCollector;
  private alerts: AlertManager;

  async start() {
    // Collect metrics
    this.collectMetrics();

    // Setup alert rules
    this.setupAlerts();
  }

  private collectMetrics() {
    // Version lag
    this.metrics.gauge('synddb_version_lag', () => {
      return this.targetVersion - this.currentVersion;
    });

    // Sync status
    this.metrics.gauge('synddb_is_syncing', () => {
      return this.isSyncing ? 1 : 0;
    });

    // Query performance
    this.metrics.histogram('synddb_query_duration_ms');
    this.metrics.counter('synddb_query_total');
    this.metrics.counter('synddb_query_errors_total');

    // Database size
    this.metrics.gauge('synddb_database_size_bytes', () => {
      return this.getDatabaseSize();
    });

    // Cache hit rate
    this.metrics.gauge('synddb_cache_hit_rate', () => {
      return this.cache.getHitRate();
    });
  }

  private setupAlerts() {
    // Alert if lag is too high
    this.alerts.addRule({
      name: 'high_version_lag',
      condition: () => this.targetVersion - this.currentVersion > 100,
      message: 'Read replica is lagging behind by more than 100 versions'
    });

    // Alert if sync is stuck
    this.alerts.addRule({
      name: 'sync_stuck',
      condition: () => this.isSyncing && this.syncDuration > 300000, // 5 minutes
      message: 'Sync has been running for more than 5 minutes'
    });

    // Alert on query errors
    this.alerts.addRule({
      name: 'high_query_error_rate',
      condition: () => this.getQueryErrorRate() > 0.01, // 1% error rate
      message: 'Query error rate is above 1%'
    });
  }
}
```

## Phase 6: TEE Validators & Settlement (Week 12-13)

### Goals
- Deploy validators as Docker containers in Google Cloud Confidential Space
- Implement dual attestation via SP1 and Lit Actions
- Build bridge message processing for validator-only operations
- Create settlement transaction system that only validators can execute

**Implementation Approach:**
- **Google Cloud Confidential Space** - Run validators as simple Docker containers with hardware-backed TEE protection
- **Docker-based Deployment** - Standard containers with SQLite embedded, no complex TEE setup required
- **Dual Attestation** - TEE attestation verified through both SP1 (ZK proof) and Lit Actions (decentralized verification)
- **Key Management** - Each container holds its own key within the TEE environment
- **Simple Operations** - Validators just run SQLite and sign transactions when needed

### Tasks

#### 6.1 Google Cloud Confidential Space Validator Setup

##### Docker Container Configuration
```dockerfile
# Dockerfile for TEE Validator
FROM rust:1.75 as builder

# Install dependencies
RUN apt-get update && apt-get install -y \
    sqlite3 \
    libsqlite3-dev \
    ca-certificates

# Build the validator binary
WORKDIR /app
COPY . .
RUN cargo build --release

# Runtime image
FROM gcr.io/confidential-space-images/base:latest

# Copy validator binary and dependencies
COPY --from=builder /app/target/release/synddb-validator /usr/local/bin/
COPY --from=builder /usr/lib/x86_64-linux-gnu/libsqlite3.so* /usr/lib/

# Set up data directory
RUN mkdir -p /data

# Run validator
ENTRYPOINT ["/usr/local/bin/synddb-validator"]
```

##### Deployment Configuration
```yaml
# confidential-space-deployment.yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: synddb-validator
spec:
  replicas: 3
  template:
    spec:
      nodeSelector:
        cloud.google.com/gke-confidential-nodes: "true"
      containers:
      - name: validator
        image: gcr.io/synddb/validator:latest
        env:
        - name: CONFIDENTIAL_SPACE
          value: "true"
        - name: ATTESTATION_SERVICE_ENDPOINT
          value: "https://confidentialcomputing.googleapis.com"
        resources:
          limits:
            memory: "32Gi"
            cpu: "8"
        volumeMounts:
        - name: data
          mountPath: /data
      volumes:
      - name: data
        persistentVolumeClaim:
          claimName: validator-data
```

##### Validator Implementation with Dual Attestation
```rust
// synddb-validator/src/main.rs
use google_confidential_space::{AttestationClient, ContainerAttestation};
use lit_sdk::{LitNodeClient, LitAction};
use sp1_sdk::{ProverClient, SP1Stdin, SP1Proof};
use alloy::primitives::Address;

pub struct ConfidentialSpaceValidator {
    // SQLite database running in the container
    database: SyndDatabase,

    // Google Confidential Space attestation
    attestation_client: AttestationClient,

    // Lit Protocol for decentralized verification
    lit_client: LitNodeClient,

    // SP1 for ZK proof generation
    sp1_prover: ProverClient,

    // Key generated and stored within TEE
    validator_key: ValidatorKey,

    // Bridge contract interface
    bridge: BridgeContract,
}

impl ConfidentialSpaceValidator {
    pub async fn new() -> Result<Self> {
        // Initialize SQLite database within the container
        let database = SyndDatabase::new("/data/synddb.sqlite")?;

        // Get Google Confidential Space attestation
        let attestation_client = AttestationClient::new()?;

        // Initialize Lit Protocol client
        let lit_client = LitNodeClient::new().await?;

        // Initialize SP1 prover
        let sp1_prover = ProverClient::new();

        // Generate validator key within TEE
        let validator_key = ValidatorKey::generate_in_tee()?;

        Ok(Self {
            database,
            attestation_client,
            lit_client,
            sp1_prover,
            validator_key,
            bridge: BridgeContract::new(config.bridge_address),
        })
    }

    pub async fn initialize(&mut self) -> Result<()> {
        // Step 1: Get Google Confidential Space attestation
        let gcp_attestation = self.attestation_client
            .get_container_attestation()
            .await?;

        // Step 2: Generate SP1 ZK proof of correct TEE setup
        let sp1_proof = self.generate_sp1_attestation_proof(&gcp_attestation).await?;

        // Step 3: Verify via Lit Actions for decentralized attestation
        let lit_verification = self.verify_with_lit_actions(&gcp_attestation, &sp1_proof).await?;

        // Step 4: Register validator on-chain only after dual verification
        self.register_validator(lit_verification).await?;

        info!("Validator registered with dual attestation verification");
        Ok(())
    }

    async fn generate_sp1_attestation_proof(&self, attestation: &ContainerAttestation) -> Result<SP1Proof> {
        // Create SP1 input with attestation data
        let mut stdin = SP1Stdin::new();
        stdin.write(&attestation.measurements);
        stdin.write(&attestation.container_id);
        stdin.write(&self.database.get_state_hash());

        // Generate ZK proof that TEE is correctly configured
        let proof = self.sp1_prover
            .prove_tee_setup(stdin)
            .await?;

        Ok(proof)
    }

    async fn verify_with_lit_actions(&self, attestation: &ContainerAttestation, sp1_proof: &SP1Proof) -> Result<LitVerification> {
        // Submit to Lit network for decentralized verification
        let verification = self.lit_client
            .verify_attestation(
                attestation.to_bytes(),
                sp1_proof.to_bytes(),
            )
            .await?;

        Ok(verification)
    }

    pub async fn process_withdrawals(&mut self) -> Result<()> {
        // Simple SQLite query within the Docker container
        let withdrawals = self.database.query(
            "SELECT * FROM withdrawal_requests
             WHERE status = 'PENDING'
             ORDER BY timestamp ASC
             LIMIT 100",
            &[]
        ).await?;

        for withdrawal in withdrawals {
            // Process each withdrawal with dual attestation
            self.process_withdrawal_with_attestation(withdrawal).await?;
        }

        Ok(())
    }

    async fn process_withdrawal_with_attestation(&mut self, withdrawal: WithdrawalRequest) -> Result<()> {
        // Step 1: Generate SP1 proof that withdrawal is valid
        let withdrawal_proof = self.generate_withdrawal_proof(&withdrawal).await?;

        // Step 2: Sign with TEE-protected key
        let signature = self.validator_key.sign(&withdrawal)?;

        // Step 3: Get fresh attestation from Confidential Space
        let attestation = self.attestation_client.get_fresh_attestation().await?;

        // Step 4: Submit to bridge with all verifications
        let tx_hash = self.bridge
            .process_withdrawal(
                withdrawal.clone(),
                withdrawal_proof,
                signature,
                attestation,
            )
            .await?;

        // Step 5: Update SQLite database
        self.database.execute(
            "UPDATE withdrawal_requests
             SET status = 'COMPLETED', settlement_tx_hash = ?1
             WHERE request_id = ?2",
            &[&tx_hash.to_string(), &withdrawal.request_id],
        ).await?;

        Ok(())
    }

    async fn generate_withdrawal_proof(&self, withdrawal: &WithdrawalRequest) -> Result<SP1Proof> {
        // Simple proof generation using SP1
        let mut stdin = SP1Stdin::new();
        stdin.write(&withdrawal.encode());
        stdin.write(&self.database.get_state_hash());

        let proof = self.sp1_prover.prove_withdrawal(stdin).await?;
        Ok(proof)
    }
}
```

##### Simplified Lit Action for Attestation Verification
```javascript
// lit-action-validator.js - Deployed to IPFS for Lit Protocol execution
const validatorLitAction = async () => {
    // This runs in Lit Protocol's distributed network
    const { attestation, sp1Proof, validatorAddress } = args;

    // Verify Google Confidential Space attestation
    const isValidAttestation = await Lit.Actions.verifyAttestation({
        attestation: attestation,
        expectedMeasurements: {
            imageDigest: "sha256:abc123...", // Expected container image
            platform: "gcp-confidential-space"
        }
    });

    // Verify SP1 proof
    const isValidProof = await Lit.Actions.verifySP1Proof({
        proof: sp1Proof,
        publicInputs: [validatorAddress]
    });

    if (isValidAttestation && isValidProof) {
        // Both attestations verified - authorize validator
        return {
            authorized: true,
            validatorAddress: validatorAddress,
            timestamp: Date.now()
        };
    }

    return { authorized: false };
};
```

##### Docker Compose for Local Testing
```yaml
# docker-compose-validator.yml
version: '3.8'

services:
  validator:
    build:
      context: .
      dockerfile: Dockerfile.validator
    environment:
      - ROLE=validator
      - CHAIN_RPC=${CHAIN_RPC}
      - BRIDGE_ADDRESS=${BRIDGE_ADDRESS}
      - LIT_NETWORK=serrano # or habanero for mainnet
      - SP1_PROVER_NETWORK=${SP1_NETWORK}
    volumes:
      - ./data:/data
    ports:
      - "8080:8080"
    # Simulate TEE environment for local testing
    security_opt:
      - seccomp:unconfined
    cap_add:
      - SYS_ADMIN
```

#### 6.2 Bridge Smart Contract
```solidity
contract SyndDBBridge {
    using ECDSA for bytes32;

    struct Validator {
        address publicKey;
        bytes attestation;
        bool isActive;
    }

    struct WithdrawalRequest {
        address recipient;
        address token;
        uint256 amount;
        uint256 nonce;
    }

    mapping(address => Validator) public validators;
    mapping(bytes32 => bool) public processedWithdrawals;

    // Circuit breakers
    uint256 public dailyWithdrawalLimit;
    uint256 public dailyWithdrawn;
    uint256 public lastWithdrawalDay;

    function processWithdrawal(
        WithdrawalRequest calldata request,
        bytes[] calldata signatures
    ) external {
        // Check circuit breakers
        require(checkWithdrawalLimits(request.amount), "Exceeds daily limit");

        // Verify signatures from validators
        bytes32 messageHash = keccak256(abi.encode(request));
        require(verifyValidatorSignatures(messageHash, signatures), "Invalid signatures");

        // Check for replay
        bytes32 withdrawalId = keccak256(abi.encode(request));
        require(!processedWithdrawals[withdrawalId], "Already processed");
        processedWithdrawals[withdrawalId] = true;

        // Execute withdrawal
        IERC20(request.token).transfer(request.recipient, request.amount);

        // Update circuit breaker counters
        updateWithdrawalCounters(request.amount);

        emit WithdrawalProcessed(request.recipient, request.token, request.amount);
    }

    function checkWithdrawalLimits(uint256 amount) internal view returns (bool) {
        uint256 currentDay = block.timestamp / 86400;

        if (currentDay != lastWithdrawalDay) {
            // New day, reset counter
            return amount <= dailyWithdrawalLimit;
        }

        return dailyWithdrawn + amount <= dailyWithdrawalLimit;
    }
}
```

## Phase 7: Testing & Optimization (Week 14-15)

### Goals
- Comprehensive testing suite (unit, integration, e2e)
- Performance optimization based on benchmarks
- Security auditing and hardening
- Documentation and deployment scripts

### Tasks

#### 7.1 Testing Suite

##### Unit Tests
```typescript
describe('SyndDatabase', () => {
  describe('Local Write Execution', () => {
    it('should execute SQL writes atomically', async () => {
      const db = new SyndDatabase(testConfig);

      await db.beginDatabaseTransaction();
      await db.execute('INSERT INTO orders VALUES (?, ?, ?, ?, ?)', params1);
      await db.execute('UPDATE balances SET balance = balance - ? WHERE account_id = ?', params2);
      await db.commit();

      const order = await db.query('SELECT * FROM orders WHERE order_id = ?', [orderId]);
      expect(order).toBeDefined();

      const balance = await db.query('SELECT balance FROM balances WHERE account_id = ?', [accountId]);
      expect(balance.balance).toBe(expectedBalance);
    });

    it('should rollback on error', async () => {
      // Test rollback behavior
    });
  });
});
```

##### Integration Tests
```typescript
describe('End-to-End Flow', () => {
  let sequencer: Sequencer;
  let replica: ReadReplica;
  let chain: MockChain;

  beforeEach(async () => {
    chain = await MockChain.deploy();
    sequencer = new Sequencer(sequencerConfig);
    replica = new ReadReplica(replicaConfig);

    await sequencer.start();
    await replica.start();
  });

  it('should sync state from sequencer to read replica', async () => {
    // Submit local writes to sequencer
    const writes = generateTestLocalWrites(100);
    for (const write of writes) {
      await sequencer.executeLocalWrite(write);
    }

    // Wait for publish
    await waitForPublish();

    // Wait for replica sync
    await waitForSync(replica);

    // Query replica and verify state
    const result = await replica.query('SELECT COUNT(*) as count FROM orders');
    expect(result[0].count).toBe(100);
  });
});
```

#### 7.2 Performance Optimization

##### Query Optimization
```sql
-- Core engine provides query optimization infrastructure
-- Applications define their specific index requirements through extensions

-- Example: Analyze query patterns
EXPLAIN QUERY PLAN
SELECT * FROM application_table
WHERE indexed_column = ?
ORDER BY sort_column DESC;

-- Extensions define indexes through SchemaExtension trait
impl SchemaExtension for MyExtension {
    fn index_statements(&self) -> Vec<String> {
        vec![
            "CREATE INDEX idx_primary_lookup ON my_table(key_column)".to_string(),
            "CREATE INDEX idx_composite ON my_table(col1, col2 DESC)".to_string(),
            "CREATE INDEX idx_partial ON my_table(status) WHERE active = 1".to_string(),
        ]
    }
}
```

##### Memory Optimization
```typescript
class MemoryOptimizer {
  optimizeForWorkload(workload: 'orderbook' | 'transfers' | 'mixed') {
    switch (workload) {
      case 'orderbook':
        // Optimize for frequent updates and range queries
        this.db.pragma('cache_size = -128000'); // 128MB
        this.db.pragma('temp_store = MEMORY');
        this.db.pragma('mmap_size = 50000000000'); // 50GB
        break;

      case 'transfers':
        // Optimize for high insert rate
        this.db.pragma('cache_size = -64000'); // 64MB
        this.db.pragma('wal_autocheckpoint = 10000');
        break;

      case 'mixed':
        // Balanced configuration
        this.db.pragma('cache_size = -96000'); // 96MB
        this.db.pragma('wal_autocheckpoint = 5000');
        break;
    }
  }
}
```

## Deployment & Production Readiness

### Infrastructure Requirements

#### Sequencer Node
- CPU: 16+ cores (for parallel transaction processing)
- RAM: 64GB+ (for in-memory caching)
- Storage: NVMe SSD, 2TB+ (for database and WAL)
- Network: 10Gbps+ (for state publishing)

#### Read Replica Node
- CPU: 8+ cores
- RAM: 32GB+
- Storage: NVMe SSD, 1TB+
- Network: 1Gbps+

### Deployment Configuration
```yaml
# docker-compose.yml
version: '3.8'

services:
  sequencer:
    image: synddb/sequencer:latest
    environment:
      - ROLE=sequencer
      - CHAIN_RPC=${CHAIN_RPC}
      - PRIVATE_KEY=${SEQUENCER_PRIVATE_KEY}
    volumes:
      - sequencer-data:/data
    ports:
      - "8080:8080"
    deploy:
      resources:
        limits:
          cpus: '16'
          memory: 64G

  replica:
    image: synddb/replica:latest
    environment:
      - ROLE=replica
      - CHAIN_RPC=${CHAIN_RPC}
    volumes:
      - replica-data:/data
    ports:
      - "3000:3000"
    deploy:
      replicas: 3
      resources:
        limits:
          cpus: '8'
          memory: 32G

volumes:
  sequencer-data:
  replica-data:
```

### Monitoring Stack
```yaml
# monitoring.yml
services:
  prometheus:
    image: prom/prometheus
    volumes:
      - ./prometheus.yml:/etc/prometheus/prometheus.yml
    ports:
      - "9090:9090"

  grafana:
    image: grafana/grafana
    ports:
      - "3001:3000"
    volumes:
      - ./dashboards:/var/lib/grafana/dashboards
```

## Success Metrics

### Performance Targets
- **Transaction Throughput**: 50,000+ TPS for simple operations
- **Query Latency**: <5ms p99 for indexed queries
- **Sync Latency**: <1 second from sequencer to read replicas
- **State Publishing Cost**: <$0.01 per 1000 transactions

### Reliability Targets
- **Uptime**: 99.9% for read replicas, 99.95% for sequencer
- **Data Durability**: 99.999999% (via blockchain + IPFS/Arweave)
- **Recovery Time**: <5 minutes from snapshot

## Risk Mitigation

### Technical Risks
1. **SQLite Scalability Limits**
   - Mitigation: Implement sharding for ultra-high scale
   - Fallback: PostgreSQL compatibility layer

2. **Network Partitions**
   - Mitigation: Multi-region deployment with failover
   - Fallback: Async replication with conflict resolution

3. **State Corruption**
   - Mitigation: Merkle proofs and checksums
   - Fallback: Rebuild from snapshots

### Operational Risks
1. **Sequencer Compromise**
   - Mitigation: Hardware security modules (HSM)
   - Mitigation: Multi-sig for critical operations
   - Mitigation: Circuit breakers and rate limits

2. **Storage Provider Failure**
   - Mitigation: Multi-provider redundancy (IPFS + Arweave)
   - Fallback: Direct chain storage for critical data

## Timeline Summary

- **Weeks 1-2**: Architecture skeleton and interfaces
- **Weeks 3-5**: SQLite engine and performance tuning
- **Weeks 6-7**: Transaction types and triggers
- **Weeks 8-9**: Blockchain integration and publishing
- **Weeks 10-11**: Read replica implementation
- **Weeks 12-13**: TEE validators (specialized read replicas) and settlement
- **Weeks 14-15**: Testing and optimization

## Next Steps

1. Review and approve implementation plan
2. Set up development environment
3. Begin Phase 1: Architecture Skeleton
4. Establish CI/CD pipeline
5. Create initial benchmarking suite

## Appendix: Technology Choices

### Why SQLite?
- Embedded database (no separate server process)
- Excellent performance for single-sequencer model
- Battle-tested with billions of deployments
- Full ACID compliance
- Rich SQL support including triggers

### Why ZSTD Compression?
- Best compression ratio for structured data
- Fast compression/decompression
- Tunable compression levels
- Wide language support

### Why IPFS/Arweave?
- IPFS: Fast retrieval, wide adoption
- Arweave: Permanent storage guarantee
- Both: Content-addressed for integrity

### Why TEEs for Settlement?
- Hardware-enforced security boundaries
- Attestable execution environment
- Key protection without key management complexity
- Lower trust requirements than pure software validators