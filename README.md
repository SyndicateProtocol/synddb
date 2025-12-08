# SyndDB - High-Performance Blockchain Database

SyndDB enables developers to build high-performance blockchain applications using **any programming language** with SQLite bindings. Applications import a lightweight client library that automatically captures and sends SQL operations to a sequencer service for publishing.

## Overview

SyndDB consists of three main components:

1. **Your Application** - Uses SQLite as normal (any language, any framework)
2. **SyndDB Client Library** - Embedded in your application, captures changesets and sends them to the sequencer
3. **SyndDB Sequencer Service** - Receives changesets from client libraries and publishes to data availability layers

Minimal integration required - just import the client library and attach to your SQLite connection.

## Documentation

- **[SPEC.md](SPEC.md)** - Complete technical specification covering architecture, terminology, and design decisions
- **[PLAN_SEQUENCER.md](PLAN_SEQUENCER.md)** - Sequencer implementation plan (Session Extension monitoring, batching, publishing)

## Repository Structure

```
SyndDB/
├── crates/
│   ├── synddb-client/       # Client library (Rust, Python, Node.js, C FFI)
│   ├── synddb-benchmark/    # Orderbook benchmark tool for sequencer development
│   └── synddb-sequencer/    # Sequencer service (planned)
├── SPEC.md                  # Full specification
├── PLAN_SEQUENCER.md       # Sequencer service implementation plan
└── README.md               # This file
```

## Quick Start

### 1. Install & Build

```bash
git clone https://github.com/anthropics/synddb
cd SyndDB
cargo build --workspace --release
```

### 2. Try the Benchmark Tool

The benchmark tool simulates realistic database workload for testing:

```bash
# Initialize database
cargo run --package synddb-benchmark --release -- init

# Run simulation at 100 ops/sec
cargo run --package synddb-benchmark --release -- run --rate 100

# View statistics
cargo run --package synddb-benchmark --release -- stats
```

See [crates/synddb-benchmark/README.md](crates/synddb-benchmark/README.md) for full documentation.

### 3. Try the Client Library

```bash
# Run example that demonstrates client library integration
cargo run --package synddb-client --example basic_usage

# Run example with schema change detection
cargo run --package synddb-client --example schema_snapshot_example
```

See [crates/synddb-client/README.md](crates/synddb-client/README.md) for integration guide.

## Key Features

- **Language Agnostic**: Works with any language that has SQLite bindings (Python, JavaScript, Go, Rust, etc.)
- **High Performance**: Sub-millisecond writes, 50,000-100,000+ ops/sec throughput
- **Deterministic Replication**: Session Extension changesets for validators
- **Automatic Publishing**: Client library automatically sends changesets to sequencer service for DA layer publishing
- **Minimal Integration**: Just import the client library and attach to your SQLite connection

## Components

### Benchmark Tool

A high-performance orderbook simulator for testing and developing the sequencer:

- **Multiple load patterns**: Continuous, burst, or auto-discover max throughput
- **Realistic operations**: Orders, trades, cancellations, balance updates
- **Performance modes**: Full mode (~2-5k ops/sec) or simple mode (~50-100k ops/sec)
- **Adaptive testing**: Automatically finds system performance limits

[Full Documentation →](crates/synddb-benchmark/README.md)

### Client Library (Implemented)

The client library embeds in applications and captures SQL changes:

- **Session Monitor**: Captures changesets via SQLite Session Extension (thread-safe architecture)
- **Changeset Sender**: Batches and sends changesets to sequencer service via HTTP
- **Snapshot Sender**: Creates and sends periodic snapshots and schema-triggered snapshots
- **TEE Attestation**: Includes GCP Confidential Space attestation tokens
- **Recovery**: Persists failed batches for retry after network issues
- **Single-Threaded**: SQLite's Session Extension is not thread-safe; all SyndDB calls must be on one thread ([details](crates/synddb-client/README.md#thread-safety))

[Client Library Documentation →](crates/synddb-client/README.md)

### Sequencer Service (Planned)

The sequencer service receives changesets from clients and publishes to DA layers:

- **HTTP Receiver**: Receives changesets/snapshots from client libraries
- **Batcher**: Accumulates and batches received changesets
- **Attestor**: Compresses and signs batches with TEE-protected keys
- **Publisher**: Publishes to multiple DA layers (Celestia, EigenDA, IPFS, Arweave)
- **Message Monitor**: Handles bidirectional bridge message passing

[Sequencer Implementation Plan →](PLAN_SEQUENCER.md)

## Use Cases

- **Decentralized Exchanges**: High-frequency orderbook matching with blockchain settlement
- **Gaming**: Real-time game state with verifiable history
- **Social Applications**: Fast local-first apps with eventual consistency
- **Financial Systems**: Low-latency trading with audit trails
- **IoT & Edge Computing**: Edge processing with centralized verification

## Performance

The benchmark tool demonstrates SQLite's capabilities:

- **Full Mode**: ~2,000-5,000 ops/sec (complex orderbook operations)
- **Simple Mode**: ~50,000-100,000+ ops/sec (insert-only stress testing)
- **Auto-discovery**: Adaptive algorithm finds maximum sustainable throughput

Performance scales with hardware and can be tuned via transaction batching, cache sizes, and SQLite optimizations.

## Development

```bash
# Build all crates
cargo build --workspace

# Run tests
cargo test --workspace

# Run benchmark with detailed logging
RUST_LOG=debug cargo run --package synddb-benchmark -- run --rate 100
```

### Run All CI Checks Locally

First, install required tools:
```bash
cargo install taplo-cli@0.9.0 cargo-machete@0.7.0 cargo-nextest
rustup toolchain install nightly
```

**Check all (CI equivalent - non-destructive):**
```bash
taplo lint "**/Cargo.toml" && taplo fmt --check "**/Cargo.toml" && cargo machete && cargo +nightly fmt --all --check && cargo clippy --workspace --all-targets --all-features && cargo nextest run --workspace --all-features && cargo test --workspace --doc
```

**Fix all issues automatically:**
```bash
taplo fmt "**/Cargo.toml" && cargo +nightly fmt --all && cargo clippy --workspace --all-targets --all-features --fix --allow-dirty --allow-staged
```
*Note: `cargo-machete` only reports unused deps - you must remove them manually from Cargo.toml files.*

**Run checks individually:**
```bash
# Cargo.toml validation and formatting check
taplo lint "**/Cargo.toml" && taplo fmt --check "**/Cargo.toml"

# Find unused dependencies (manual removal required)
cargo machete

# Rust formatting check
cargo +nightly fmt --all --check

# Linting
cargo clippy --workspace --all-targets --all-features

# Tests
cargo nextest run --workspace --all-features
cargo test --workspace --doc
```

## Architecture

SyndDB uses SQLite's Session Extension to capture row-level changes deterministically. The client library:

1. Attaches to your SQLite database via Session Extension
2. Detects changes via SQLite update hooks
3. Extracts changesets when `publish()` is called (automatically every 1 second, or manually)
4. Batches changesets and sends to sequencer service via HTTP
5. Creates periodic snapshots and snapshots on schema changes
6. Includes TEE attestation tokens with all data

The sequencer service receives changesets from clients, compresses and signs them, then publishes to configured DA layers.

Applications continue using SQLite normally - the client library operates transparently in the background. For critical transactions, applications can call `publish()` immediately after committing.

## Requirements

- Rust 1.90.0 or later
- SQLite 3.x (bundled with rusqlite)

## License

MIT License - see LICENSE file for details
