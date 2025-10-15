# synddb-core

The foundational infrastructure for SyndDB - a high-performance blockchain database system.

## Overview

`synddb-core` provides the core abstractions and implementations for building blockchain databases with ultra-low latency (<1ms) local writes and decentralized state replication. It uses SQLite as the execution engine, optimized for high-throughput blockchain workloads.

## Architecture

SyndDB follows a **Core + Extensions** architecture:

- **Core** (this crate): Provides database execution, state replication, and blockchain integration infrastructure
- **Extensions**: Implement business logic, schemas, triggers, and bridge operations on top of the Core

### Key Components

```
synddb-core/
├── database.rs     - SQLite abstraction with performance optimizations
├── types.rs        - Core type definitions and error handling
├── config.rs       - Configuration management system
├── extensions.rs   - Extension registry and trait definitions
└── writes.rs       - Local write processing and queueing
```

## Features

- **High-Performance SQLite**: Optimized for 50,000+ TPS with WAL mode, 2GB cache, 256GB mmap
- **Automatic Statement Caching**: Transparent prepared statement optimization (no manual registration)
- **Real-Time Metrics**: Latency percentiles, throughput, and error rate tracking
- **Extension System**: Clean trait-based extensibility for custom business logic
- **Async/Await**: Full tokio async support for non-blocking operations
- **State Replication**: Snapshot and diff generation for read replica synchronization
- **Type Safety**: Comprehensive Rust type system with strong error handling
- **Connection Pooling**: Efficient r2d2-based connection management

## Quick Start

### Basic Usage

```rust
use synddb_core::{
    database::SqliteDatabase,
    config::SyndDBConfig,
    extensions::ExtensionRegistry,
    types::SqlValue,
};
use std::sync::Arc;

#[tokio::main]
async fn main() -> synddb_core::types::Result<()> {
    // Load configuration
    let config = SyndDBConfig::test_config();

    // Initialize database with connection pool
    let database = Arc::new(SqliteDatabase::new(
        &config.database.path,
        config.database.pool_size,
    )?);

    // Execute SQL
    database.execute(
        "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)",
        vec![]
    ).await?;

    // Insert data
    database.execute(
        "INSERT INTO users (name) VALUES (?1)",
        vec![SqlValue::Text("Alice".to_string())]
    ).await?;

    // Query data
    let results = database.query(
        "SELECT * FROM users WHERE name = ?1",
        vec![SqlValue::Text("Alice".to_string())]
    ).await?;

    println!("Found {} users", results.row_count);

    Ok(())
}
```

### Building Extensions

Extensions implement one or more trait interfaces:

#### Schema Extension

```rust
use synddb_core::extensions::SchemaExtension;
use synddb_core::types::Result;

struct MySchema;

impl SchemaExtension for MySchema {
    fn schema_id(&self) -> &str {
        "my_app"
    }

    fn version(&self) -> u32 {
        1
    }

    fn create_statements(&self) -> Vec<String> {
        vec![
            "CREATE TABLE orders (
                id INTEGER PRIMARY KEY,
                user_id INTEGER NOT NULL,
                amount INTEGER NOT NULL,
                status TEXT NOT NULL
            )".to_string()
        ]
    }

    fn index_statements(&self) -> Vec<String> {
        vec![
            "CREATE INDEX idx_orders_user ON orders(user_id)".to_string(),
            "CREATE INDEX idx_orders_status ON orders(status)".to_string(),
        ]
    }

    fn migrate_statements(&self, from_version: u32) -> Result<Vec<String>> {
        Ok(Vec::new())
    }

    fn seed_statements(&self) -> Vec<String> {
        Vec::new()
    }
}
```

#### Local Write Extension

```rust
use synddb_core::extensions::LocalWriteExtension;
use synddb_core::types::{Result, SqlOperation, SqlValue};
use async_trait::async_trait;

struct PlaceOrderWrite;

#[async_trait]
impl LocalWriteExtension for PlaceOrderWrite {
    fn write_type(&self) -> &str {
        "place_order"
    }

    fn schema(&self) -> &serde_json::Value {
        &serde_json::json!({
            "type": "object",
            "required": ["user_id", "amount"],
            "properties": {
                "user_id": {"type": "integer"},
                "amount": {"type": "integer"}
            }
        })
    }

    fn validate(&self, request: &serde_json::Value) -> Result<()> {
        // Validation logic
        Ok(())
    }

    fn to_sql(&self, request: &serde_json::Value) -> Result<Vec<SqlOperation>> {
        let user_id = request["user_id"].as_i64().unwrap();
        let amount = request["amount"].as_i64().unwrap();

        Ok(vec![SqlOperation {
            sql: "INSERT INTO orders (user_id, amount, status) VALUES (?1, ?2, ?3)"
                .to_string(),
            params: vec![
                SqlValue::Integer(user_id),
                SqlValue::Integer(amount),
                SqlValue::Text("PENDING".to_string()),
            ],
        }])
    }
}
```

#### Using Extensions

```rust
use synddb_core::extensions::ExtensionRegistry;
use std::sync::Arc;

// Create registry
let mut registry = ExtensionRegistry::new();

// Register extensions
registry.register_schema(Arc::new(MySchema))?;
registry.register_write(Arc::new(PlaceOrderWrite))?;

// Initialize all extensions (creates tables, indexes, triggers)
registry.initialize(database.as_ref()).await?;

// Extensions are now active
```

## Extension Traits

The Core provides five extension traits:

| Trait | Purpose | Example Use Cases |
|-------|---------|-------------------|
| `SchemaExtension` | Define database tables and indexes | Order books, balances, leaderboards |
| `LocalWriteExtension` | Define custom write operations | Place order, transfer tokens, update score |
| `TriggerExtension` | Define SQLite triggers for business logic | Order matching, balance validation, auto-liquidation |
| `BridgeExtension` | Define cross-chain bridge operations | Deposits, withdrawals, cross-chain messaging |
| `QueryExtension` | Define optimized query patterns | Leaderboards, order book depth, user balances |

## Performance Optimizations

The Core applies aggressive SQLite optimizations:

```rust
// WAL mode for concurrent reads during writes
PRAGMA journal_mode = WAL;

// NORMAL sync - durability to OS, periodic blockchain commits
PRAGMA synchronous = NORMAL;

// 2GB cache for hot data
PRAGMA cache_size = -2000000;

// 256GB memory map for large databases
PRAGMA mmap_size = 274877906944;

// 64KB pages for reduced B-tree depth
PRAGMA page_size = 65536;

// EXCLUSIVE locking (single sequencer)
PRAGMA locking_mode = EXCLUSIVE;
```

## Configuration

SyndDB uses a layered configuration system with the following priority order (highest to lowest):

1. **Environment variables** (prefixed with `SYNDDB_`)
2. **Configuration files** (YAML/TOML/JSON)
3. **Default values** (defined in code)

### Development Setup

For local development, create a `.env` file from the example:

```bash
cp .env.example .env
# Edit .env with your local settings
```

The `.env` file is automatically loaded during development and should **never** be committed to version control.

### Configuration File

Create a `config.yaml` file:

```yaml
synddb:
  role: sequencer

  database:
    path: ./data/synddb.sqlite
    pool_size: 16
    journal_mode: WAL
    synchronous: NORMAL
    cache_size: -2000000
    mmap_size: 274877906944

  sequencer:
    batch_size: 1000
    batch_timeout_ms: 100
    publish_interval_ms: 1000
    max_diff_size: 1048576
    blockchain_snapshot_interval: 10000

  chain:
    rpc_url: https://rpc.syndicate.io
    contract_address: "0x..."
    # Optional fee caps for protection (omit to use Alloy's estimates)
    # max_priority_fee_per_gas: 2  # Cap priority fee (MEV protection)
    # max_fee_per_gas: 100  # Cap total fee (gas spike protection)
    confirmations: 1  # Blocks to wait (higher = more secure)
    poll_interval_ms: 1000  # Transaction status polling interval

  storage:
    provider: ipfs
    ipfs:
      gateway: https://ipfs.io
      api_endpoint: http://localhost:5001
```

### Environment Variables

You can override any config value using environment variables:

```bash
# Override database path
export SYNDDB_DATABASE_PATH=/custom/path/db.sqlite

# Override RPC URL
export SYNDDB_CHAIN_RPC_URL=https://custom-rpc.example.com

# Private keys should ONLY be set via env vars, never in config files
export SYNDDB_CHAIN_PRIVATE_KEY=0x...
```

### Loading Configuration

```rust
// Loads from config file + .env (if present) + environment variables
let config = SyndDBConfig::from_file("config.yaml")?;
config.validate()?;
```

### Production Deployment

In production:
- **DO NOT** use `.env` files (they won't exist)
- Set environment variables via your deployment platform (Docker, Kubernetes, systemd, etc.)
- Use secret management for sensitive values (private keys, API keys)
- Configuration files can be used for non-sensitive defaults

## Testing

Run the test suite:

```bash
# Run all tests
cargo test --package synddb-core

# Run specific test
cargo test --package synddb-core test_database_creation

# Run with output
cargo test --package synddb-core -- --nocapture
```

Current test coverage:
- 13 unit tests across all modules
- Database operations (CRUD, transactions)
- Configuration loading and validation
- Extension registry
- Write queueing and validation
- Type system and utilities

## Performance Targets

Based on PLAN_CORE.md, the system targets:

- **Transaction Throughput**: 50,000+ TPS for simple operations
- **Query Latency**: <5ms p99 for indexed queries
- **Local Write Latency**: <1ms (no distributed consensus)
- **State Publishing Cost**: <$0.01 per 1000 transactions

## Architecture Diagrams

### Local Write Flow

```
Client Request
    ↓
execute_local_write()
    ↓
Extension Validation
    ↓
Convert to SQL
    ↓
Execute in Local SQLite (<1ms)
    ↓
Queue for Blockchain Submission
    ↓
Return Receipt
```

### State Replication Flow

```
Sequencer (Single Node)
    ↓
Batch Local Writes
    ↓
Generate Diff/Snapshot
    ↓
Compress (zstd)
    ↓
Submit to Blockchain
    ↓
Read Replicas (Anyone)
    ↓
Fetch & Decompress
    ↓
Apply to Local SQLite
    ↓
Serve Queries
```

## API Documentation

Generate and view API documentation:

```bash
cargo doc --package synddb-core --open
```

## Development Status

✅ **Phase 1 Complete**: Architecture skeleton and core infrastructure
- Core traits and types
- SQLite database implementation
- Configuration system
- Extension registry
- Local write framework

🚧 **Phase 2 In Progress**: SQLite performance optimization
- Prepared statement caching
- Performance benchmarking suite
- Query optimization

📋 **Upcoming Phases**:
- Phase 3: Trigger system and business logic
- Phase 4: Blockchain integration
- Phase 5: Read replica implementation
- Phase 6: TEE validators and settlement

## License

MIT

## Related Documentation

- [Implementation Plan](../../PLAN_CORE.md) - Detailed implementation roadmap
- [Architecture Spec](../../SPEC.md) - System architecture and design
- [Extension Guide](../../PLAN_EXTENSIONS.md) - Guide to building extensions

## Contributing

This is the foundational crate for SyndDB. Changes should maintain:
- Backward compatibility with existing extensions
- Performance characteristics (50k+ TPS target)
- Clean separation between Core and extension concerns
- Comprehensive test coverage
