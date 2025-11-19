# synddb-sequencer

Sequencer service that receives changesets from SyndDB client libraries and publishes them to data availability layers.

## Overview

The `synddb-sequencer` is a Rust service that:

1. **Receives changesets and snapshots** from application client libraries via HTTP
2. **Verifies TEE attestation** tokens from client applications
3. **Batches changesets** efficiently with configurable size and time thresholds
4. **Compresses and signs** batches with TEE-protected keys
5. **Publishes to multiple DA layers** (Celestia, EigenDA, IPFS, Arweave)
6. **Handles message passing** for inbound deposits and outbound withdrawals

## Status

🚧 **Planned Implementation** - Directory structure and interfaces defined, implementation planned.

See [PLAN_SEQUENCER.md](../../PLAN_SEQUENCER.md) for detailed architecture and implementation plan.

## Architecture

The sequencer runs as a **service in a separate TEE** from the application. It receives changesets and snapshots via HTTP from client libraries embedded in applications, then publishes them to DA layers.

### Components

- **HTTP Receiver**: Receives changesets/snapshots from client libraries via HTTP
- **Attestation Verifier**: Verifies client TEE attestation tokens
- **Batcher**: Accumulates received changesets and snapshots
- **Attestor**: Compresses data with zstd and signs with secp256k1
- **Publisher**: Publishes to configured DA/storage layers with retry logic
- **Message Monitor**: Handles bidirectional message passing with blockchain

## Usage

### Basic Usage

```bash
# Run sequencer service with default configuration
synddb-sequencer --config default.yaml

# Use custom configuration
synddb-sequencer --config custom.yaml
```

### Configuration

See `config/example.yaml` for all configuration options:

```yaml
receiver:
  listen_addr: "0.0.0.0:8433"
  enable_tls: true
  verify_client_attestation: true

batch:
  max_batch_size: 1048576  # 1MB
  max_batch_age: "1s"

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

### HTTP Receiver

Receives changesets and snapshots from client libraries:

- **TEE Attestation Verification**: Verifies client attestation tokens
- **Changeset Validation**: Validates sequence numbers and checksums
- **Backpressure Handling**: Manages queue depth and flow control
- **Health Monitoring**: Tracks client connection status

### Changeset Processing

Processes changesets received from clients:

- **Deterministic Format**: Changesets use SQLite Session Extension format (INSERT/UPDATE/DELETE with values)
- **Compact Representation**: Only changed rows, not full database pages
- **Validator-friendly**: Exact same changes can be replayed by validators
- **Auditable**: Changesets can be inspected to see what changed

### Snapshot Handling

Handles snapshots received from clients:

- **Periodic Snapshots**: Recovery points sent by clients every hour
- **Schema Change Snapshots**: Full snapshots triggered by DDL operations
- **Audit Trail**: Schema changes include DDL statements
- **Validator Bootstrapping**: Enables validators to reconstruct complete state

### Multi-DA Publishing

Publishes to multiple DA layers in parallel:

- **Celestia**: Modular DA layer
- **EigenDA**: Ethereum-based DA
- **IPFS**: Content-addressed storage
- **Arweave**: Permanent storage

### Message Passing

Bidirectional message passing for bridge operations:

- **Inbound**: Monitors blockchain for deposit events, delivers to applications via HTTP
- **Outbound**: Receives outbound messages from client libraries (withdrawal requests)
- **HTTP API**: Delivers inbound messages to applications (localhost:8432)

## Development Status

### Implemented ✅

- Directory structure with all modules stubbed out
- Module interfaces and types defined
- Configuration system with YAML support
- CLI argument parsing
- Cargo.toml with all dependencies

### TODO ⬜

- HTTP receiver implementation (Axum endpoints)
- Client attestation verification
- Changeset batching logic
- TEE attestation and signing
- DA layer publisher implementations
- Message passing system
- Integration tests with synddb-client

## Development Workflow

```bash
# Terminal 1: Run application with synddb-client
cargo run --package synddb-client --example basic_usage

# Terminal 2: Run sequencer service - planned
cargo run --package synddb-sequencer -- --config default.yaml
```

## Design Goals

- **Minimal Integration**: Just import client library and attach to SQLite connection
- **Language Agnostic**: Client library available for Rust, Python, Node.js, and C FFI
- **High Performance**: Minimal overhead on application performance
- **Deterministic Replication**: Session Extension changesets for validators
- **Automatic Publishing**: Handles all DA layer interaction
- **Separate TEE Architecture**: Runs in separate TEE from application for key isolation

## References

- [PLAN_SEQUENCER.md](../../PLAN_SEQUENCER.md) - Detailed architecture and implementation plan
- [SPEC.md](../../SPEC.md) - Overall SyndDB specification
- [SQLite Session Extension](https://www.sqlite.org/sessionintro.html) - Official documentation
