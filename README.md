# SyndDB - High-Performance Blockchain Database

SyndDB enables developers to build high-performance blockchain applications using **any programming language** with SQLite bindings. The sequencer (running as a sidecar process) automatically captures and publishes SQL operations for verification and replication.

## Overview

SyndDB consists of two main components:

1. **Your Application** - Uses SQLite as normal (any language, any framework)
2. **SyndDB Sequencer** - Monitors changes and publishes to data availability layers (runs as a sidecar process)

No code changes required - just run your app with the sequencer.

## Documentation

- **[SPEC.md](SPEC.md)** - Complete technical specification covering architecture, terminology, and design decisions
- **[PLAN_SEQUENCER.md](PLAN_SEQUENCER.md)** - Sequencer implementation plan (Session Extension monitoring, batching, publishing)

## Repository Structure

```
SyndDB/
├── crates/
│   ├── synddb-benchmark/    # Orderbook benchmark tool for sequencer development
│   └── synddb-sequencer/    # Sequencer (coming soon)
├── SPEC.md                  # Full specification
├── PLAN_SEQUENCER.md       # Sequencer architecture plan
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

### 3. Run the Sequencer (Coming Soon)

```bash
# Terminal 1: Run your application (or benchmark)
cargo run --package synddb-benchmark -- run --rate 100

# Terminal 2: Run sequencer (sidecar process)
cargo run --package synddb-sequencer -- --db orderbook.db
```

## Key Features

- **Language Agnostic**: Works with any language that has SQLite bindings (Python, JavaScript, Go, Rust, etc.)
- **High Performance**: Sub-millisecond writes, 50,000-100,000+ ops/sec throughput
- **Deterministic Replication**: Session Extension changesets for validators
- **Automatic Publishing**: Sequencer (sidecar process) handles all DA layer interaction
- **Zero Code Changes**: Drop-in solution for existing SQLite applications

## Components

### Benchmark Tool

A high-performance orderbook simulator for testing and developing the sequencer:

- **Multiple load patterns**: Continuous, burst, or auto-discover max throughput
- **Realistic operations**: Orders, trades, cancellations, balance updates
- **Performance modes**: Full mode (~2-5k ops/sec) or simple mode (~50-100k ops/sec)
- **Adaptive testing**: Automatically finds system performance limits

[Full Documentation →](crates/synddb-benchmark/README.md)

### Sequencer (Coming Soon)

The sequencer (running as a sidecar process) monitors SQLite changes and publishes them to data availability layers:

- **Session Monitor**: Attach to SQLite via Session Extension
- **Batcher**: Accumulate changesets and create periodic snapshots
- **Attestor**: Compress and sign batches with TEE-protected keys
- **Publisher**: Publish to multiple DA layers (Celestia, EigenDA, IPFS, Arweave)

[Implementation Plan →](PLAN_SEQUENCER.md)

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

## Architecture

SyndDB uses SQLite's Session Extension to capture row-level changes deterministically. The sequencer:

1. Attaches to your SQLite database
2. Captures changesets using Session Extension
3. Batches changes with periodic snapshots
4. Compresses and signs batches
5. Publishes to configured DA layers

Applications continue using SQLite normally - the sequencer operates transparently in the background as a sidecar process.

## Requirements

- Rust 1.90.0 or later
- SQLite 3.x (bundled with rusqlite)

## License

MIT License - see LICENSE file for details
