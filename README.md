# SyndDB - High-Performance Blockchain Database

SyndDB enables developers to build high-performance blockchain applications using **any programming language** with SQLite bindings. Applications import a lightweight client library that automatically captures SQL operations and sends them to a sequencer service for publishing to data availability layers.

## Overview

SyndDB consists of three main components:

1. **Your Application** - Uses SQLite as normal (any language, any framework), runs inside a TEE
2. **SyndDB Client Library** - Embedded in your application, captures changesets and sends them to the sequencer
3. **SyndDB Sequencer Service** - Receives changesets from clients and publishes to data availability layers
4. **SyndDB Validator** - Verifies SQL operations and signs for settlement

All components run inside TEEs (Google Cloud Confidential Space) with hardware-backed attestation.

## Documentation

- **[SPEC.md](SPEC.md)** - Complete technical specification covering architecture, terminology, and design decisions
- **[PLAN_SEQUENCER.md](PLAN_SEQUENCER.md)** - Sequencer implementation details
- **[PLAN_VALIDATOR.md](PLAN_VALIDATOR.md)** - Validator implementation details
- **[PLAN_BRIDGE.md](PLAN_BRIDGE.md)** - Bridge contract and message passing

## Repository Structure

```
SyndDB/
├── crates/
│   ├── synddb-client/        # Client library (Rust + FFI for Python/Node.js)
│   ├── synddb-sequencer/     # Sequencer service
│   ├── synddb-validator/     # Validator service
│   ├── synddb-storage/       # Storage layer abstractions (GCS, local)
│   ├── synddb-chain-monitor/ # Blockchain event monitoring
│   ├── synddb-shared/        # Shared types and utilities
│   ├── synddb-relayer/       # Message relayer service
│   ├── synddb-bootstrap/     # TEE bootstrap and attestation
│   ├── synddb-benchmark/     # Orderbook simulator for testing
│   ├── synddb-fuzzer/        # Fuzzing infrastructure
│   └── gcp-attestation/      # GCP Confidential Space attestation
├── contracts/                # Solidity contracts (Bridge, attestation verification)
├── examples/
│   ├── price-oracle/         # Python price oracle example
│   └── prediction-market/    # Rust prediction market example
├── deploy/terraform/         # GCP infrastructure as code
├── docker/                   # Dockerfiles for services
└── tests/                    # E2E and integration tests
```

## Prerequisites

```bash
# Rust (via rustup)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup toolchain install nightly  # Required for formatting

# Command runner
brew install just

# Solidity development
curl -L https://foundry.paradigm.xyz | bash
foundryup
```

<details>
<summary>Additional CI tools (for running full checks locally)</summary>

```bash
brew install taplo
cargo install cargo-machete cargo-nextest
```

</details>

## Development Workflow

**Primary workflow:** Most development relies on CI and auto-deploys to staging. Push to a branch and CI will:
- Run all tests and lints
- Build reproducible Docker images
- Deploy to staging (GCP Confidential Space)

This is the preferred approach since it tests the actual TEE environment and ZK proofs.

**Local development** is supported for individual components:

```bash
# Clone & build
git clone https://github.com/SyndicateProtocol/synddb
cd synddb
cargo build --workspace --release

# Run all CI checks locally
just check

# Run tests
just test

# See all commands
just
```

### Docker Compose (Local Testing)

For quick local testing without TEE features:

```bash
# Start sequencer + validator + sample app
docker compose up --build

# Test endpoints
curl http://localhost:8433/health   # Sequencer
curl http://localhost:8080/health   # Validator
```

### CI Validation

Check workflow status after pushing:

```bash
gh run list --limit 5              # Recent runs
gh run watch <run-id>              # Watch in progress
gh pr checks                       # PR status
```

## Key Features

- **Language Agnostic**: Works with any language that has SQLite bindings (Python, JavaScript, Go, Rust, etc.)
- **High Performance**: Sub-millisecond writes, 50,000-100,000+ ops/sec throughput
- **TEE Security**: Hardware-backed attestation via GCP Confidential Space
- **Deterministic Replication**: SQLite Session Extension changesets for validators
- **CBOR/COSE Wire Format**: Efficient binary encoding with cryptographic signatures

## Components

### Client Library

Embedded in applications to capture SQL changes:
- Captures changesets via SQLite Session Extension
- Batches and sends to sequencer via HTTP
- Creates periodic snapshots and schema-triggered snapshots
- Includes TEE attestation tokens

[Documentation](crates/synddb-client/README.md)

### Sequencer Service

Receives changesets from clients and publishes to storage:
- HTTP API for receiving changesets/snapshots
- Batches and signs with TEE-protected keys
- Publishes to GCS (with Celestia, EigenDA planned)
- CBOR/COSE message format

### Validator Service

Verifies SQL operations before settlement:
- Syncs from sequencer or storage layers
- Replays changesets to rebuild state
- Verifies signatures and invariants
- Signs valid states for settlement

### Smart Contracts

Solidity contracts for on-chain settlement:
- **Bridge.sol** - Cross-chain message passing and withdrawals
- **Attestation verification** - TEE attestation via SP1 ZK proofs

[Contract Documentation](contracts/README.md)

## Examples

### Price Oracle (Python)

A price oracle that fetches prices from external APIs and publishes them on-chain:

```bash
just examples::price-oracle
```

[Full Documentation](examples/price-oracle/README.md)

### Prediction Market (Rust)

A prediction market demonstrating full SyndDB integration:

```bash
just examples::prediction-market
```

## Infrastructure

### GCP Deployment

Infrastructure is managed via Terraform:

```bash
# See deploy/terraform/environments/ for staging/prod configs
cd deploy/terraform/environments/staging
terraform init
terraform plan
```

### Reproducible Builds

For TEE attestation, binaries must be reproducible:

```bash
cargo build --profile reproducible
```

## Architecture

```
Application (TEE #1)          Sequencer (TEE #2)           Validators (TEE)
       │                            │                           │
       │  SQLite + Client Library   │                           │
       ├────────────────────────────▶   Batch & Sign            │
       │     HTTP + Attestation     │       │                   │
       │                            │       ▼                   │
       │                            │  Storage (GCS/DA)         │
       │                            │       │                   │
       │                            │       └───────────────────▶  Verify & Sign
       │                            │                           │
       │                            │                           ▼
       │                            │                     Bridge.sol
```

**TEE Isolation**: Application and sequencer run in separate TEEs. This prevents the application from accessing signing keys.

## Requirements

- **Rust 1.90.0+** with nightly toolchain
- **just** command runner
- **Docker** (for local testing)
- **Foundry** (for contract development)
- **Terraform** (for infrastructure deployment)

## License

MIT License - see LICENSE file for details
