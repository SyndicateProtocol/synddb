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

## Prerequisites

Install the required tools:

```bash
# Rust (via rustup)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup toolchain install nightly  # Required for formatting

# Command runner and git hooks
brew install just lefthook

# Solidity development (optional, for smart contracts)
curl -L https://foundry.paradigm.xyz | bash
foundryup
```

<details>
<summary>Additional CI tools (for running full checks locally)</summary>

```bash
# TOML formatter and unused dependency checker
brew install taplo
cargo install cargo-machete cargo-nextest
```

</details>

## Quick Start

### 1. Clone & Build

```bash
git clone https://github.com/SyndicateProtocol/synddb
cd synddb
cargo build --workspace --release

# Install git hooks (recommended)
just hooks-install
```

### 2. Local Development

Use `just` for all development tasks:

```bash
# Start full dev environment (Anvil + contracts + sequencer)
just dev

# Or start components individually:
just deploy     # Deploy contracts to Anvil
just sequencer  # Run sequencer
just validator  # Run validator

# See all commands
just
```

**Manual approach** (without just):
```bash
./scripts/deploy-local.sh          # Deploy contracts
./scripts/dev-env.sh               # Start full environment
./scripts/dev-env.sh --validator   # Include validator
```

Deployed addresses are deterministic and defined in `.env.defaults`:
- MockWETH: `0x5FbDB2315678afecb367f032d93F642f64180aa3`
- Bridge: `0xCf7Ed3AccA5a467e9e704C703E8D87F634fB0Fc9`
- PriceOracle: `0xDc64a140Aa3E981100a9becA4E685f962f0cF6C9`

### 3. Try the Benchmark Tool (Optional)

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

### 4. Try the Client Library

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
- **Automatic Pushing**: Client library automatically sends changesets to sequencer service for storage layer publishing
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

The sequencer service receives changesets from clients and publishes to storage layers:

- **HTTP Receiver**: Receives changesets/snapshots from client libraries
- **Batcher**: Accumulates and batches received changesets
- **Attestor**: Compresses and signs batches with TEE-protected keys
- **Publisher**: Publishes to multiple storage layers (Celestia, EigenDA, IPFS, Arweave)
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
just              # Show all available commands
just build        # Build all crates
just test         # Run tests
just check        # Run all lints (fmt, clippy, deps, toml)
just fmt          # Auto-fix formatting issues
just fix          # Auto-fix all fixable issues

# Run benchmark with detailed logging
RUST_LOG=debug cargo run --package synddb-benchmark -- run --rate 100
```

### Available Just Commands

| Command | Description |
|---------|-------------|
| `just dev` | Start full dev environment |
| `just test` | Run unit tests |
| `just check` | Run all lints |
| `just fmt` | Format all code |
| `just lint` | Run Clippy |
| `just docs` | Build and open documentation |
| `just watch` | Watch mode with live rebuild |
| `just outdated` | Show outdated dependencies |
| `just audit` | Security vulnerability scan |

Run `just` to see all available commands grouped by category.

## Architecture

SyndDB uses SQLite's Session Extension to capture row-level changes deterministically. The client library:

1. Attaches to your SQLite database via Session Extension
2. Detects changes via SQLite update hooks
3. Extracts changesets when `push()` is called (automatically every 1 second, or manually)
4. Batches changesets and sends to sequencer service via HTTP
5. Creates periodic snapshots and snapshots on schema changes
6. Includes TEE attestation tokens with all data

The sequencer service receives changesets from clients, compresses and signs them, then publishes to configured storage layers.

Applications continue using SQLite normally - the client library operates transparently in the background. For critical transactions, applications can call `push()` immediately after committing.

## Requirements

- **Rust 1.90.0+** with nightly toolchain (for formatting)
- **just** and **lefthook** (see [Prerequisites](#prerequisites))
- **Docker** (optional, for E2E tests)
- **Foundry** (optional, for smart contract development)

## License

MIT License - see LICENSE file for details
