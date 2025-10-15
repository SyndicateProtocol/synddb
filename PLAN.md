# SyndDB Implementation Plan

## Executive Summary
SyndDB is a high-performance blockchain database that replaces traditional EVM execution with SQLite, enabling ultra-low latency database operations while maintaining decentralized validation. The system consists of a single sequencer node and multiple read replica nodes that anyone can run permissionlessly. Only a small subset of read replicas with TEE hardware become validators for settlement operations. This plan outlines a phased approach to build the complete system in **Rust**, starting with core architecture, focusing on SQLite performance, and progressively adding blockchain integration and validation capabilities.

## Architecture Overview

### System Components
```
┌─────────────────────────────────────────────────────────────────┐
│                         SyndDB System                            │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  ┌──────────────────┐      ┌────────────────────────────┐      │
│  │  Sequencer (1)   │      │ Read Replicas (Anyone)    │      │
│  │                   │      │                            │      │
│  │  ┌─────────────┐  │      │  ┌─────────────────────┐  │      │
│  │  │   SQLite    │  │      │  │  SQLite Replicas    │  │      │
│  │  │   Engine    │  │      │  │  (Multiple Nodes)   │  │      │
│  │  └─────────────┘  │      │  └─────────────────────┘  │      │
│  │                   │      │                            │      │
│  │  ┌─────────────┐  │      │  ┌─────────────────────┐  │      │
│  │  │  Tx Handler │  │      │  │   State Sync &      │  │      │
│  │  │  & Triggers │  │      │  │   Query Engines     │  │      │
│  │  └─────────────┘  │      │  └─────────────────────┘  │      │
│  │                   │      │                            │      │
│  │  ┌─────────────┐  │      │  ┌─────────────────────┐  │      │
│  │  │ Diff/Snap   │  │      │  │  Subset: TEE        │  │      │
│  │  │  Generator  │  │      │  │  Validators Only    │  │      │
│  │  └─────────────┘  │      │  └─────────────────────┘  │      │
│  └──────────────────┘      └────────────────────────────┘      │
│           │                            ▲                         │
│           │                            │                         │
│           ▼                            │                         │
│  ┌──────────────────────────────────────────────────────┐      │
│  │            Syndicate Chain Smart Contracts            │      │
│  │  (writeDiff, writeSnapshot, pointers, ordering)       │      │
│  └──────────────────────────────────────────────────────┘      │
│                                                                  │
│  ┌──────────────────────────────────────────────────────┐      │
│  │              Off-chain Storage (IPFS/Arweave)         │      │
│  └──────────────────────────────────────────────────────┘      │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
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
- **rusqlite** - SQLite bindings with full feature support
- **sqlx** - Async SQL toolkit (alternative to rusqlite for async contexts)
- **r2d2** - Database connection pooling
- **sled** - Embedded database for metadata storage

#### Networking & Async
- **tokio** - Async runtime for networking and I/O
- **axum** or **actix-web** - Web framework for API endpoints
- **tonic** - gRPC framework for inter-node communication
- **quinn** - QUIC implementation for low-latency networking

#### Blockchain Integration
- **ethers-rs** - Ethereum/EVM interaction
- **alloy** - Next-gen Ethereum library (alternative to ethers)
- **revm** - Ethereum Virtual Machine implementation in Rust

#### Compression & Serialization
- **zstd** - Zstandard compression
- **bincode** - Binary serialization
- **serde** - Serialization framework
- **prost** - Protocol Buffers

#### Cryptography & Security
- **ring** - Cryptographic operations
- **sha3** - SHA-3 and Keccak hashing
- **ed25519-dalek** - Ed25519 signatures
- **sgx** or **teaclave-sgx-sdk** - Intel SGX for TEE support

#### Storage Backends
- **ipfs-api** - IPFS client
- **arweave-rs** - Arweave client (community crate)

#### Utilities
- **tracing** - Structured logging and diagnostics
- **metrics** - Performance metrics collection
- **clap** - Command-line argument parsing
- **config** - Configuration management
- **thiserror** & **anyhow** - Error handling

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
├── Cargo.toml              # Workspace configuration
├── crates/
│   ├── synddb-core/        # Core database abstractions
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── database.rs    # SQLite abstraction layer
│   │       ├── types.rs       # Core type definitions
│   │       ├── config.rs      # Configuration management
│   │       └── lib.rs
│   ├── synddb-sequencer/   # Sequencer implementation
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── main.rs        # Sequencer binary
│   │       ├── sequencer.rs   # Main sequencer orchestrator
│   │       ├── batcher.rs     # Transaction batching logic
│   │       ├── compressor.rs  # State diff/snapshot compression
│   │       └── publisher.rs   # Blockchain publishing
│   ├── synddb-replica/     # Read replica implementation
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── main.rs        # Replica binary
│   │       ├── replica.rs     # Main read replica orchestrator
│   │       ├── syncer.rs      # State sync
│   │       ├── reconstructor.rs # State reconstruction
│   │       ├── query.rs       # Query interface
│   │       └── validator.rs   # Optional: TEE validator
│   ├── synddb-contracts/   # Smart contract bindings
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── abi/           # Contract ABIs
│   │       ├── bindings.rs    # Generated bindings (ethers-rs)
│   │       └── lib.rs
│   ├── synddb-storage/     # Storage backends
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── ipfs.rs        # IPFS integration
│   │       ├── arweave.rs     # Arweave integration
│   │       └── lib.rs
│   └── synddb-utils/       # Shared utilities
│       ├── Cargo.toml
│       └── src/
│           ├── compression.rs # Compression utilities
│           ├── hashing.rs     # Hashing utilities
│           ├── metrics.rs     # Metrics collection
│           └── lib.rs
├── contracts/              # Solidity contracts
│   ├── src/
│   └── test/
├── tests/                  # Integration tests
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
    async fn execute(&self, sql: &str, params: &[&dyn rusqlite::ToSql]) -> Result<ExecuteResult>;
    async fn begin_transaction(&self) -> Result<DatabaseTransaction>;
    async fn generate_snapshot(&self) -> Result<StateSnapshot>;
    async fn generate_diff(&self, from_version: u64, to_version: u64) -> Result<StateDiff>;
    async fn apply_snapshot(&self, snapshot: StateSnapshot) -> Result<()>;
    async fn apply_diff(&self, diff: StateDiff) -> Result<()>;
}

// synddb-sequencer/src/sequencer.rs
#[async_trait]
pub trait Sequencer: Send + Sync {
    async fn start(&mut self) -> Result<()>;
    async fn stop(&mut self) -> Result<()>;
    async fn submit_transaction(&self, tx: DatabaseTransaction) -> Result<TransactionReceipt>;
    async fn publish_state(&self) -> Result<PublishReceipt>;
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
use ethers::types::H256;

#[async_trait]
pub trait ChainPublisher: Send + Sync {
    async fn publish_diff(&self, diff: &[u8]) -> Result<H256>;
    async fn publish_diff_pointer(&self, cid: &str) -> Result<H256>;
    async fn publish_snapshot(&self, snapshot: &[u8]) -> Result<H256>;
    async fn publish_snapshot_pointer(&self, cid: &str) -> Result<H256>;
}
```

#### 1.3 Configuration System
```yaml
# config.yaml
synddb:
  role: sequencer  # or replica

  database:
    path: ./data/synddb.sqlite
    journal_mode: WAL
    synchronous: NORMAL
    cache_size: -64000  # 64MB
    mmap_size: 30000000000  # 30GB

  sequencer:
    batch_size: 1000
    batch_timeout_ms: 100
    compression: zstd
    publish_interval_ms: 1000
    max_diff_size: 1048576  # 1MB
    state_snapshot_interval: 10000  # Every 10k transactions

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

    fn initialize_optimizations(conn: &Connection) -> Result<()> {
        // Performance-critical pragmas
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "cache_size", -64000)?;
        conn.pragma_update(None, "mmap_size", 30000000000i64)?;
        conn.pragma_update(None, "temp_store", "MEMORY")?;
        conn.pragma_update(None, "locking_mode", "EXCLUSIVE")?;
        conn.pragma_update(None, "page_size", 4096)?;

        info!("SQLite optimizations applied");
        Ok(())
    }

    pub async fn execute_in_batch(&self, transactions: Vec<SqlTransaction>) -> Result<BatchResult> {
        let start = Instant::now();
        let conn = self.pool.get()?;

        let tx = conn.transaction()?;
        let mut results = Vec::new();

        for sql_tx in transactions.iter() {
            match self.execute_single(&tx, sql_tx) {
                Ok(result) => results.push(result),
                Err(e) => {
                    tx.rollback()?;
                    return Err(e);
                }
            }
        }

        tx.commit()?;

        let duration = start.elapsed();
        histogram!("synddb.batch.duration", duration);
        counter!("synddb.batch.transactions", transactions.len() as u64);

        Ok(BatchResult {
            success: true,
            results,
            duration,
        })
    }
}
```

#### 2.2 Schema Design for Common Use Cases

##### Order Book Schema
```sql
-- Core order book tables
CREATE TABLE orders (
  order_id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL,
  side TEXT CHECK(side IN ('BUY', 'SELL')),
  price REAL NOT NULL,
  quantity REAL NOT NULL,
  remaining_quantity REAL NOT NULL,
  status TEXT CHECK(status IN ('OPEN', 'PARTIAL', 'FILLED', 'CANCELED')),
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  nonce INTEGER NOT NULL
);

CREATE INDEX idx_orders_status_price ON orders(status, side, price);
CREATE INDEX idx_orders_account ON orders(account_id, status);

CREATE TABLE trades (
  trade_id TEXT PRIMARY KEY,
  buy_order_id TEXT NOT NULL,
  sell_order_id TEXT NOT NULL,
  price REAL NOT NULL,
  quantity REAL NOT NULL,
  timestamp INTEGER NOT NULL,
  FOREIGN KEY (buy_order_id) REFERENCES orders(order_id),
  FOREIGN KEY (sell_order_id) REFERENCES orders(order_id)
);

-- ERC-20 token tables
CREATE TABLE token_metadata (
  token_address TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  symbol TEXT NOT NULL,
  decimals INTEGER NOT NULL,
  total_supply TEXT NOT NULL
);

CREATE TABLE balances (
  account_id TEXT NOT NULL,
  token_address TEXT NOT NULL,
  balance TEXT NOT NULL,
  nonce INTEGER NOT NULL,
  PRIMARY KEY (account_id, token_address),
  FOREIGN KEY (token_address) REFERENCES token_metadata(token_address)
);

CREATE TABLE transfer_events (
  event_id TEXT PRIMARY KEY,
  token_address TEXT NOT NULL,
  from_address TEXT NOT NULL,
  to_address TEXT NOT NULL,
  amount TEXT NOT NULL,
  timestamp INTEGER NOT NULL,
  block_number INTEGER,
  transaction_index INTEGER
);

-- Bridge/settlement tables
CREATE TABLE withdrawal_requests (
  request_id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL,
  token_address TEXT NOT NULL,
  amount TEXT NOT NULL,
  destination_address TEXT NOT NULL,
  status TEXT CHECK(status IN ('PENDING', 'PROCESSING', 'COMPLETED', 'FAILED')),
  timestamp INTEGER NOT NULL,
  settlement_tx_hash TEXT
);

CREATE TABLE deposit_records (
  deposit_id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL,
  token_address TEXT NOT NULL,
  amount TEXT NOT NULL,
  source_tx_hash TEXT NOT NULL,
  timestamp INTEGER NOT NULL
);
```

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
        let scenarios = vec![
            Scenario {
                name: "High-frequency order book".to_string(),
                setup: Box::new(|db| Self::create_order_book_schema(db)),
                workload: Box::new(|| Self::generate_order_book_transactions(10000)),
                target_tps: 50000,
                target_latency_p99_ms: 5,
            },
            Scenario {
                name: "Token transfers".to_string(),
                setup: Box::new(|db| Self::create_token_schema(db)),
                workload: Box::new(|| Self::generate_token_transfers(10000)),
                target_tps: 100000,
                target_latency_p99_ms: 2,
            },
            Scenario {
                name: "Complex analytical queries".to_string(),
                setup: Box::new(|db| Self::create_analytics_schema(db)),
                workload: Box::new(|| Self::generate_analytical_queries(100)),
                target_tps: 1000,
                target_latency_p99_ms: 50,
            },
        ];

        Self { db, scenarios }
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
            let transactions = generate_test_transactions(100);
            db.execute_in_batch(black_box(transactions))
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

## Phase 3: Transaction Type System & SQLite Triggers (Week 6-7)

### Goals
- Build flexible transaction type system using SQLite triggers
- Implement validation and business logic at database level
- Create programmable hooks for custom transaction types
- Build transaction serialization and deserialization

### Tasks

#### 3.1 Transaction Type Registry (Rust)
```rust
// synddb-core/src/transaction_types.rs
use serde::{Serialize, Deserialize};
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use rusqlite::Connection;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TransactionType {
    // Order book operations
    PlaceOrder,
    CancelOrder,
    MatchOrders,

    // Token operations
    Transfer,
    Mint,
    Burn,

    // Bridge operations
    Deposit,
    Withdraw,

    // Custom operations
    Custom(String),
}

pub trait TransactionDefinition: Send + Sync {
    fn transaction_type(&self) -> TransactionType;
    fn version(&self) -> u32;
    fn schema(&self) -> &JsonValue;
    fn validate(&self, tx: &JsonValue) -> Result<()>;
    fn serialize(&self, tx: &JsonValue) -> Result<Vec<u8>>;
    fn deserialize(&self, data: &[u8]) -> Result<JsonValue>;
    fn generate_sql(&self, tx: &JsonValue) -> Result<Vec<String>>;
}

pub struct TransactionTypeRegistry {
    definitions: Arc<RwLock<HashMap<String, Box<dyn TransactionDefinition>>>>,
}

impl TransactionTypeRegistry {
    pub fn new() -> Self {
        Self {
            definitions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn register(&self, definition: Box<dyn TransactionDefinition>) -> Result<()> {
        let key = format!("{}:{}",
            serde_json::to_string(&definition.transaction_type())?,
            definition.version()
        );

        let mut defs = self.definitions.write();
        defs.insert(key, definition);

        // Install triggers after registering
        self.install_triggers(&*definition)?;

        Ok(())
    }

    fn install_triggers(&self, definition: &dyn TransactionDefinition) -> Result<()> {
        let trigger_sql = self.generate_trigger_sql(definition)?;

        // Execute trigger installation
        let conn = self.get_connection()?;
        conn.execute_batch(&trigger_sql)?;

        Ok(())
    }
}
```

#### 3.2 SQLite Trigger System

##### Order Matching Trigger
```sql
-- Trigger for automatic order matching when new order is placed
CREATE TRIGGER match_orders_on_insert
AFTER INSERT ON orders
WHEN NEW.status = 'OPEN'
BEGIN
  -- Find matching orders on opposite side
  WITH matches AS (
    SELECT
      order_id,
      price,
      remaining_quantity,
      MIN(NEW.remaining_quantity, remaining_quantity) as match_quantity
    FROM orders
    WHERE side != NEW.side
      AND status IN ('OPEN', 'PARTIAL')
      AND (
        (NEW.side = 'BUY' AND price <= NEW.price) OR
        (NEW.side = 'SELL' AND price >= NEW.price)
      )
    ORDER BY
      price ASC,  -- Best price first
      created_at ASC  -- Time priority
    LIMIT 1
  )
  INSERT INTO trades (trade_id, buy_order_id, sell_order_id, price, quantity, timestamp)
  SELECT
    hex(randomblob(16)),
    CASE WHEN NEW.side = 'BUY' THEN NEW.order_id ELSE order_id END,
    CASE WHEN NEW.side = 'SELL' THEN NEW.order_id ELSE order_id END,
    price,
    match_quantity,
    strftime('%s', 'now')
  FROM matches
  WHERE match_quantity > 0;

  -- Update matched orders
  UPDATE orders
  SET
    remaining_quantity = remaining_quantity - (
      SELECT match_quantity FROM matches WHERE matches.order_id = orders.order_id
    ),
    status = CASE
      WHEN remaining_quantity = 0 THEN 'FILLED'
      ELSE 'PARTIAL'
    END,
    updated_at = strftime('%s', 'now')
  WHERE order_id IN (
    SELECT order_id FROM matches
    UNION SELECT NEW.order_id
  );
END;
```

##### Balance Validation Trigger
```sql
-- Trigger to validate sufficient balance before transfer
CREATE TRIGGER validate_balance_on_transfer
BEFORE UPDATE ON balances
WHEN NEW.balance < 0
BEGIN
  SELECT RAISE(ABORT, 'Insufficient balance for transfer');
END;

-- Trigger to update total supply on mint/burn
CREATE TRIGGER update_supply_on_balance_change
AFTER UPDATE ON balances
BEGIN
  UPDATE token_metadata
  SET total_supply = (
    SELECT SUM(CAST(balance AS INTEGER))
    FROM balances
    WHERE token_address = NEW.token_address
  )
  WHERE token_address = NEW.token_address;
END;
```

##### Withdrawal Request Processing
```sql
-- Trigger to mark tokens as locked when withdrawal is requested
CREATE TRIGGER lock_tokens_on_withdrawal
AFTER INSERT ON withdrawal_requests
BEGIN
  -- Deduct from user's balance
  UPDATE balances
  SET balance = CAST(balance AS INTEGER) - CAST(NEW.amount AS INTEGER)
  WHERE account_id = NEW.account_id
    AND token_address = NEW.token_address;

  -- Create audit log entry
  INSERT INTO transfer_events (event_id, token_address, from_address, to_address, amount, timestamp)
  VALUES (
    hex(randomblob(16)),
    NEW.token_address,
    NEW.account_id,
    '0x0000000000000000000000000000000000000000',  -- Burn address
    NEW.amount,
    NEW.timestamp
  );
END;
```

#### 3.3 Transaction Builder Pattern (Rust)
```rust
// synddb-core/src/builders.rs
use crate::transaction_types::TransactionType;
use uuid::Uuid;
use std::time::SystemTime;

pub struct TransactionBuilder;

impl TransactionBuilder {
    pub fn place_order() -> OrderTransactionBuilder {
        OrderTransactionBuilder::new()
    }

    pub fn transfer() -> TransferTransactionBuilder {
        TransferTransactionBuilder::new()
    }
}

#[derive(Default)]
pub struct OrderTransactionBuilder {
    account_id: Option<String>,
    side: Option<OrderSide>,
    price: Option<f64>,
    quantity: Option<f64>,
}

#[derive(Debug, Clone)]
pub enum OrderSide {
    Buy,
    Sell,
}

impl OrderTransactionBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn account(mut self, account_id: impl Into<String>) -> Self {
        self.account_id = Some(account_id.into());
        self
    }

    pub fn side(mut self, side: OrderSide) -> Self {
        self.side = Some(side);
        self
    }

    pub fn price(mut self, price: f64) -> Self {
        self.price = Some(price);
        self
    }

    pub fn quantity(mut self, quantity: f64) -> Self {
        self.quantity = Some(quantity);
        self
    }

    pub fn build(self) -> Result<DatabaseTransaction> {
        // Validate required fields
        let account_id = self.account_id
            .ok_or_else(|| anyhow!("Missing account_id"))?;
        let side = self.side
            .ok_or_else(|| anyhow!("Missing side"))?;
        let price = self.price
            .ok_or_else(|| anyhow!("Missing price"))?;
        let quantity = self.quantity
            .ok_or_else(|| anyhow!("Missing quantity"))?;

        // Generate SQL
        let sql = r#"
            INSERT INTO orders (order_id, account_id, side, price, quantity,
                              remaining_quantity, status, created_at, updated_at, nonce)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        "#;

        let order_id = Uuid::new_v4().to_string();
        let timestamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)?
            .as_secs();

        Ok(DatabaseTransaction {
            transaction_type: TransactionType::PlaceOrder,
            sql: sql.to_string(),
            params: vec![
                order_id.clone().into(),
                account_id.into(),
                format!("{:?}", side).into(),
                price.into(),
                quantity.into(),
                quantity.into(), // remaining = initial quantity
                "OPEN".into(),
                timestamp.into(),
                timestamp.into(),
                generate_nonce().into(),
            ],
            metadata: serde_json::json!({
                "order_id": order_id,
                "account_id": account_id,
                "side": side,
                "price": price,
                "quantity": quantity,
            }),
        })
    }
}

fn generate_nonce() -> u64 {
    use rand::Rng;
    rand::thread_rng().gen()
}
```

## Phase 4: Blockchain Integration & State Publishing (Week 8-9)

### Goals
- Implement smart contracts for state publication
- Build state diff and state snapshot generation system
- Implement compression and chunking for large states
- Create off-chain storage integration (IPFS/Arweave)

### Tasks

#### 4.1 Smart Contract Implementation
```solidity
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract SyndDB {
    struct StateCommitment {
        uint256 blockNumber;
        uint256 timestamp;
        bytes32 stateRoot;
        uint256 version;
        address publisher;
    }

    struct DiffCommitment {
        bytes32 diffHash;
        uint256 fromVersion;
        uint256 toVersion;
        uint256 chunkCount;
        string storagePointer; // IPFS CID or Arweave ID
    }

    struct SnapshotCommitment {
        bytes32 snapshotHash;
        uint256 version;
        uint256 chunkCount;
        string storagePointer;
    }

    // State variables
    mapping(uint256 => StateCommitment) public states;
    mapping(uint256 => DiffCommitment) public diffs;
    mapping(uint256 => SnapshotCommitment) public snapshots;

    uint256 public currentVersion;
    address public sequencer;

    // Events
    event DiffPublished(uint256 indexed fromVersion, uint256 indexed toVersion, bytes32 diffHash);
    event SnapshotPublished(uint256 indexed version, bytes32 snapshotHash);
    event StateAdvanced(uint256 indexed version, bytes32 stateRoot);

    modifier onlySequencer() {
        require(msg.sender == sequencer, "Only sequencer can publish");
        _;
    }

    // Publish state diff directly to chain (for small state diffs)
    function publishDiff(
        bytes32 diffHash,
        uint256 diffIndex,
        bytes calldata diffData
    ) external onlySequencer {
        // Store diff chunk
        // Implementation depends on max size constraints
    }

    // Publish state diff pointer (for large state diffs stored off-chain)
    function publishDiffPointer(
        bytes32 diffHash,
        uint256 fromVersion,
        uint256 toVersion,
        string calldata storagePointer
    ) external onlySequencer {
        require(toVersion > fromVersion, "Invalid version range");
        require(toVersion > currentVersion, "Version must advance");

        diffs[currentVersion] = DiffCommitment({
            diffHash: diffHash,
            fromVersion: fromVersion,
            toVersion: toVersion,
            chunkCount: 0,
            storagePointer: storagePointer
        });

        currentVersion = toVersion;
        emit DiffPublished(fromVersion, toVersion, diffHash);
    }

    // Publish state snapshot directly to chain
    function publishSnapshot(
        bytes32 snapshotHash,
        uint256 snapshotIndex,
        bytes calldata snapshotData
    ) external onlySequencer {
        // Store snapshot chunk
    }

    // Publish state snapshot pointer
    function publishSnapshotPointer(
        bytes32 snapshotHash,
        uint256 version,
        string calldata storagePointer
    ) external onlySequencer {
        snapshots[version] = SnapshotCommitment({
            snapshotHash: snapshotHash,
            version: version,
            chunkCount: 0,
            storagePointer: storagePointer
        });

        emit SnapshotPublished(version, snapshotHash);
    }
}
```

#### 4.2 State Diff Generation System (Rust)
```rust
// synddb-sequencer/src/diff_generator.rs
use serde::{Serialize, Deserialize};
use sha3::{Digest, Sha3_256};
use zstd;

pub struct DiffGenerator {
    last_published_version: u64,
    change_log: ChangeLog,
}

impl DiffGenerator {
    pub async fn generate_diff(&self, from_version: u64, to_version: u64) -> Result<StateDiff> {
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

        // Create state diff object
        let diff = StateDiff {
            from_version,
            to_version,
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

        Ok(StateDiff {
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
class IPFSStorage implements IStorageProvider {
  private ipfs: IPFS;

  async store(data: Buffer): Promise<string> {
    // Add to IPFS with chunking for large files
    const options = {
      pin: true,
      chunker: 'rabin-avg-65536'
    };

    const result = await this.ipfs.add(data, options);
    return result.cid.toString();
  }

  async retrieve(cid: string): Promise<Buffer> {
    const chunks: Buffer[] = [];

    for await (const chunk of this.ipfs.cat(cid)) {
      chunks.push(chunk);
    }

    return Buffer.concat(chunks);
  }
}

class ArweaveStorage implements IStorageProvider {
  private arweave: Arweave;

  async store(data: Buffer): Promise<string> {
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

  async retrieve(id: string): Promise<Buffer> {
    const data = await this.arweave.transactions.getData(id, { decode: true });
    return Buffer.from(data);
  }
}
```

#### 4.5 State Publisher
```typescript
class StatePublisher {
  private queue: PublishQueue;
  private storage: IStorageProvider;
  private chain: IChainPublisher;

  async publishState() {
    const batch = await this.queue.getBatch();

    if (batch.transactions.length === 0) {
      return;
    }

    // Generate state diff from last published version
    const diff = await this.diffGenerator.generateDiff(
      this.lastPublishedVersion,
      batch.version
    );

    // Decide whether to publish directly or via pointer
    const publishStrategy = this.determinePublishStrategy(diff);

    if (publishStrategy === 'direct') {
      await this.publishDirect(diff);
    } else {
      await this.publishViaPointer(diff);
    }

    // Update last published version
    this.lastPublishedVersion = batch.version;

    // Check if snapshot is needed
    if (this.shouldCreateSnapshot(batch.version)) {
      await this.publishSnapshot(batch.version);
    }
  }

  private determinePublishStrategy(diff: StateDiff): 'direct' | 'pointer' {
    const MAX_ONCHAIN_SIZE = 100 * 1024; // 100KB
    return diff.compressedSize < MAX_ONCHAIN_SIZE ? 'direct' : 'pointer';
  }

  private async publishDirect(diff: StateDiff) {
    // Chunk if necessary
    const chunks = this.chunkData(diff.compressed, 30000);

    for (let i = 0; i < chunks.length; i++) {
      await this.chain.publishDiff(chunks[i]);
    }
  }

  private async publishViaPointer(diff: StateDiff) {
    // Store to IPFS/Arweave
    const cid = await this.storage.store(diff.compressed);

    // Publish pointer to chain
    await this.chain.publishDiffPointer(cid);
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
  private syncer: StateSyncer;
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
class StateSyncer {
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

  private async applyDiff(diffCommitment: DiffCommitment) {
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
    const diff = JSON.parse(decompressed.toString()) as StateDiff;

    // Verify integrity
    const calculatedHash = hash(decompressed);
    if (calculatedHash !== diffCommitment.diffHash) {
      throw new Error('Diff integrity check failed');
    }

    // Apply SQL statements
    await this.db.executeInBatch(diff.statements);
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
- Implement TEE-based validators (a small subset of read replicas with settlement capabilities)
- Build bridge message processing for validator-only operations
- Create settlement transaction system that only validators can execute
- Implement circuit breakers and safety mechanisms

**Key Distinctions:**
- **Validators are optional** - the system works with just regular read replicas for queries
- **Only a small subset of read replicas become validators** - most replicas remain query-only nodes
- **TEE hardware required** - validators need specialized TEE hardware that regular replicas don't
- **Settlement authority** - only validators can sign bridge transactions, regular replicas cannot
- **Higher trust requirements** - validators must be registered and attested, replicas need no permission

### Tasks

#### 6.1 TEE Validator Architecture (Specialized Read Replica)
```typescript
// TEEValidator extends ReadReplicaNode with additional settlement capabilities
class TEEValidator extends ReadReplicaNode {
  private enclave: SecureEnclave;
  private settlementKey: PrivateKey;
  private bridge: BridgeContract;

  async initialize() {
    // Initialize secure enclave
    this.enclave = await SecureEnclave.create({
      attestation: true,
      sealing: true
    });

    // Generate or unseal settlement key
    this.settlementKey = await this.enclave.getOrGenerateKey('settlement');

    // Register with bridge contract
    await this.registerWithBridge();
  }

  private async registerWithBridge() {
    // Generate attestation report
    const attestation = await this.enclave.generateAttestation({
      userData: this.settlementKey.publicKey
    });

    // Submit to bridge contract
    await this.bridge.registerValidator(
      this.settlementKey.publicKey,
      attestation
    );
  }

  async processWithdrawals() {
    // Query pending withdrawals
    const withdrawals = await this.db.query(`
      SELECT * FROM withdrawal_requests
      WHERE status = 'PENDING'
      ORDER BY timestamp ASC
      LIMIT 100
    `);

    for (const withdrawal of withdrawals) {
      await this.processWithdrawal(withdrawal);
    }
  }

  private async processWithdrawal(withdrawal: WithdrawalRequest) {
    // Validate withdrawal
    if (!await this.validateWithdrawal(withdrawal)) {
      await this.markWithdrawalFailed(withdrawal);
      return;
    }

    // Sign withdrawal message
    const message = this.encodeWithdrawalMessage(withdrawal);
    const signature = await this.enclave.sign(message, this.settlementKey);

    // Submit to bridge
    const tx = await this.bridge.processWithdrawal(
      withdrawal,
      signature
    );

    // Update status
    await this.db.execute(`
      UPDATE withdrawal_requests
      SET status = 'COMPLETED', settlement_tx_hash = ?
      WHERE request_id = ?
    `, [tx.hash, withdrawal.request_id]);
  }
}
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
  describe('Transaction Execution', () => {
    it('should execute transactions atomically', async () => {
      const db = new SyndDatabase(testConfig);

      await db.beginTransaction();
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
    // Submit transactions to sequencer
    const txs = generateTestTransactions(100);
    for (const tx of txs) {
      await sequencer.submitTransaction(tx);
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
-- Analyze query patterns and create appropriate indexes
EXPLAIN QUERY PLAN
SELECT * FROM orders
WHERE status = 'OPEN' AND side = 'BUY'
ORDER BY price DESC, created_at ASC;

-- Create covering index for common queries
CREATE INDEX idx_orders_open_orders
ON orders(status, side, price DESC, created_at ASC)
WHERE status = 'OPEN';

-- Partial indexes for better performance
CREATE INDEX idx_orders_active
ON orders(account_id, status, updated_at)
WHERE status IN ('OPEN', 'PARTIAL');
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