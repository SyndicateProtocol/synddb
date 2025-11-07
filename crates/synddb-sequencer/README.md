# synddb-sequencer

Lightweight SQLite monitor and publisher that runs as a sidecar process alongside your application.

## Overview

The `synddb-sequencer` is a zero-configuration Rust process that:

1. **Monitors SQLite databases** using the SQLite Session Extension to capture deterministic changesets
2. **Batches changesets** efficiently with configurable size and time thresholds
3. **Compresses and signs** batches with TEE attestation support
4. **Publishes to multiple DA layers** (Celestia, EigenDA, IPFS, Arweave)
5. **Handles message passing** for inbound deposits and outbound withdrawals

## Status

🚧 **Stub Implementation** - Directory structure and interfaces defined, implementation in progress.

See [PLAN_SEQUENCER.md](../../PLAN_SEQUENCER.md) for detailed architecture and implementation plan.

## Architecture

The sequencer runs as a **sidecar process** - a separate process that runs alongside your application. It has read-only access to the SQLite database and automatically captures all changes without requiring any application code changes.

### Components

- **Session Monitor**: Attaches to SQLite via Session Extension to capture logical changes
- **Batcher**: Accumulates changesets and creates periodic snapshots
- **Attestor**: Compresses data with zstd and signs with secp256k1
- **Publisher**: Publishes to configured DA/storage layers with retry logic
- **Message Monitor**: Handles bidirectional message passing with blockchain

## Usage

### Basic Usage

```bash
# Monitor a database with default configuration
synddb-sequencer --db /path/to/app.db

# Use custom configuration
synddb-sequencer --db /path/to/app.db --config custom.yaml
```

### Configuration

See `config/example.yaml` for all configuration options:

```yaml
database:
  path: "app.db"
  enable_sessions: true

batch:
  max_batch_size: 1048576  # 1MB
  max_batch_age: "1s"
  snapshot_interval: "1h"
  snapshot_threshold: 1000

publish:
  celestia:
    rpc_url: "http://localhost:26658"
    namespace: "synddb"
  # ... other DA layers

messages:
  enable_inbound: true
  enable_outbound: true
  api_port: 8432
  chain_rpc_url: "http://localhost:8545"
  bridge_contract: "0x..."

tee:
  enable_attestation: true
  gcp_project_id: "my-project"
  key_path: "/secrets/signing-key"
```

## Key Features

### SQLite Session Extension

Uses the official SQLite Session Extension API (not WAL parsing) for:

- **Deterministic capture**: Logical changes (INSERT/UPDATE/DELETE with values)
- **Compact format**: Only changed rows, not full database pages
- **Validator-friendly**: Exact same changes can be replayed by validators
- **Auditable**: Changesets can be inspected to see what changed

### Schema Change Tracking

Automatically creates full snapshots when schema changes are detected:

- Monitors `sqlite_schema` table for DDL operations
- Triggers immediate snapshot on ALTER TABLE, CREATE TABLE, etc.
- Includes DDL statements in snapshot for audit trail
- Ensures validators can always reconstruct complete state

### Multi-DA Publishing

Publishes to multiple DA layers in parallel:

- **Celestia**: Modular DA layer
- **EigenDA**: Ethereum-based DA
- **IPFS**: Content-addressed storage
- **Arweave**: Permanent storage

### Message Passing

Bidirectional message passing for bridge operations:

- **Inbound**: Monitors blockchain for deposit events
- **Outbound**: Monitors SQLite tables for withdrawal requests
- **HTTP API**: Delivers messages to application (localhost:8432)

## Development Status

### Implemented ✅

- Directory structure with all modules stubbed out
- Module interfaces and types defined
- Configuration system with YAML support
- CLI argument parsing
- Cargo.toml with all dependencies

### TODO ⬜

- Session Monitor implementation (SQLite Session Extension integration)
- Changeset batching logic
- TEE attestation and signing
- DA layer publisher implementations
- Message passing system
- Integration tests

## Development Workflow

```bash
# Terminal 1: Run benchmark to generate database activity
cargo run --package synddb-benchmark -- run --rate 100

# Terminal 2: Run sequencer (sidecar process) - stub for now
cargo run --package synddb-sequencer -- --db orderbook.db
```

## Design Goals

- **Zero Code Changes**: Drop-in solution for existing SQLite applications
- **Language Agnostic**: Works with any language that has SQLite bindings
- **High Performance**: Minimal overhead on application performance
- **Deterministic Replication**: Session Extension changesets for validators
- **Automatic Publishing**: Handles all DA layer interaction
- **Sidecar Architecture**: Runs as a separate process alongside your application

## References

- [PLAN_SEQUENCER.md](../../PLAN_SEQUENCER.md) - Detailed architecture and implementation plan
- [SPEC.md](../../SPEC.md) - Overall SyndDB specification
- [SQLite Session Extension](https://www.sqlite.org/sessionintro.html) - Official documentation
