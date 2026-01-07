# SyndDB Development Summary

This document provides an overview of SyndDB's development progression, key architectural decisions, and testing readiness status for team members reviewing the codebase.

## Summary

SyndDB is a SQLite replication system that enables high-performance blockchain applications. Applications write to SQLite normally while a client library captures changes and sends them to a sequencer service, which publishes to data availability layers. Validators verify the SQL operations and sign for on-chain settlement.

**Current Status**: Core infrastructure complete. Staging environment deployed on GCP Confidential Space with Base Sepolia integration. Examples demonstrate functionality. Onchain infrastructure has been deployed, but we're still wiring up the TEEs to the Bridge contract.

**Current Goal**: Complete TEE bootstrapping to enable sequencers and validators to register their keys on-chain and write to the Bridge contract. This is the critical path to production and lets us test the examples fully.

---

## Component Overview

### SQLite Change Capture: Changesets vs WAL

SyndDB currently uses **changesets** (via SQLite Session Extension) to capture database modifications:

- **Changesets**: Logical operations (INSERT/UPDATE/DELETE) with old and new values. Captured at the SQL level, deterministic, and compact for replication.
- **WAL (Write-Ahead Log)**: Physical page-level changes to the database file. Lower-level, includes internal SQLite bookkeeping.

We're still prototyping whether changesets or WAL parsing provides the optimal tradeoff between simplicity, performance, and compatibility. Changesets are the current implementation.

### 1. Client Library (`crates/synddb-client/`)

Lightweight library embedded in applications to capture SQLite changesets.

| Aspect        | Details                                                                       |
| ------------- | ----------------------------------------------------------------------------- |
| **Purpose**   | Attach to SQLite, capture changesets via Session Extension, send to sequencer |
| **Languages** | Rust (native), Python, Node.js, Go (via C FFI)                                |
| **Key Files** | `lib.rs`, `session.rs`, `sender.rs`, `bindings/`                              |
| **Status**    | Production-ready                                                              |

**What it does**:

- Attaches SQLite Session Extension to capture INSERT/UPDATE/DELETE
- Background thread batches and sends changesets to sequencer
- Creates periodic snapshots for recovery
- Includes TEE attestation tokens when running in Confidential Space

**What it does NOT do**:

- Sign changesets (no keys in application TEE)
- Publish to storage layers (sequencer's job)

### 2. Sequencer (`crates/synddb-sequencer/`)

Central service that receives changesets and publishes to storage.

| Aspect        | Details                                                |
| ------------- | ------------------------------------------------------ |
| **Purpose**   | Receive, batch, sign, and publish changesets/snapshots |
| **Transport** | HTTP API on port 8433                                  |
| **Storage**   | GCS (primary), local filesystem (dev)                  |
| **Key Files** | `http_api.rs`, `batcher.rs`, `transport/`, `messages/` |
| **Status**    | Production-ready                                       |

**Endpoints**:

- `POST /submit` - Receive changesets
- `POST /snapshot` - Receive snapshots
- `GET /batches/{start}_{end}` - Retrieve batch data
- `GET/POST /messages/inbound`, `/messages/outbound` - Message passing

### 3. Validator (`crates/synddb-validator/`)

Verifies SQL operations before signing for settlement.

| Aspect        | Details                                                                        |
| ------------- | ------------------------------------------------------------------------------ |
| **Purpose**   | Sync from sequencer/GCS, replay changesets, verify invariants, sign for bridge |
| **Fetchers**  | HTTP (sequencer API), GCS (batch storage)                                      |
| **Key Files** | `sync/`, `apply/`, `bridge/`, `rules/`                                         |
| **Status**    | Core complete, TEE integration pending                                         |

**Validation Flow**:

1. Fetch batches from sequencer or GCS
2. Verify sequencer signatures
3. Apply changesets to local SQLite replica
4. Run validation rules
5. Sign valid state updates for bridge

### 4. Smart Contracts (`contracts/src/`)

On-chain components for message passing and attestation verification.

| Contract                  | Purpose                                                 |
| ------------------------- | ------------------------------------------------------- |
| `Bridge.sol`              | Cross-chain message execution with validator signatures |
| `AttestationVerifier.sol` | Verify RISC Zero proofs of GCP attestation tokens       |
| `TeeKeyManager.sol`       | Register/deregister TEE public keys                     |
| `GasTreasury.sol`         | Gas sponsorship for relayer transactions                |
| `ModuleCheckRegistry.sol` | Pluggable validation modules                            |

**Deployed**: Base Sepolia testnet at `0x7ddE947a7DF980fD00887371C527cBbcB4E92a4E`

### 5. Supporting Crates

| Crate                  | Purpose                                                    |
| ---------------------- | ---------------------------------------------------------- |
| `synddb-bootstrap`     | TEE key generation and attestation proof submission        |
| `synddb-relayer`       | Submit signed messages to bridge on behalf of TEE services |
| `synddb-chain-monitor` | WebSocket subscription to on-chain events                  |
| `synddb-shared`        | Common types (CBOR messages, COSE signatures)              |
| `synddb-storage`       | Storage layer abstractions (GCS, local)                    |
| `gcp-attestation`      | Parse and verify Confidential Space OIDC tokens            |

---

## Key Architectural Decisions

### 1. Two-VM TEE Isolation

**Decision**: Application and sequencer run in separate TEEs.

**Rationale**: The sequencer holds signing keys for storage layer publishing. Running in a separate TEE prevents the application from accessing these keys, even if application code is compromised.

```
TEE #1 (Application)          TEE #2 (Sequencer)
┌─────────────────────┐       ┌─────────────────────┐
│ Application Code    │       │ Signing Keys        │
│ synddb-client       │──────▶│ Batcher             │
│ No signing keys     │ HTTP  │ Storage Publisher   │
└─────────────────────┘       └─────────────────────┘
```

### 2. SQL Operations as Audit Trail

**Decision**: Capture SQL changesets rather than requiring deterministic re-execution.

**Rationale**: Applications can use any language, framework, or external API. Validators verify the database operations, not the original business logic. This enables practical verification without restricting application development.

### 3. RISC Zero for TEE Attestation

**Decision**: Use RISC Zero zkVM (replacing SP1) for on-chain attestation verification.

**Rationale**: SP1 has a closed-source GPU prover that cannot run in Google Cloud Run due to a mandated use of Docker-in-Docker and memory requirements that exceed Cloud Run. Network proving adds setup burden and gets expensive quite quickly. RISC Zero has a fully open-source GPU prover that can potentially run in Cloud Run, and provides cost-effective proving on self-hosted GPU infrastructure.

**Evolution**: Started with SP1 SDK, migrated to RISC Zero 3.0 for Cloud Run compatibility and open-source GPU prover availability.

### 4. GCS as Primary Storage

**Decision**: Google Cloud Storage for batch persistence, with Arweave/Celestia as future options.

**Rationale**: GCS integrates naturally with Confidential Space workload identity. Permanent storage (Arweave) deprioritized until required for production.

### 5. CBOR/COSE Wire Format

**Decision**: Binary encoding with cryptographic signatures.

**Rationale**:

- ~40% size reduction vs JSON+base64
- COSE_Sign1 provides standard authenticated containers
- Content-addressable via SHA-256 hashes
- Compatible with future Arweave ANS-104 wrapping

### 6. secp256k1 Image Signing

**Decision**: Sign Docker images with secp256k1 keys (same curve as Ethereum).

**Rationale**: Enables on-chain verification via ecrecover. Image signatures are attached as OCI artifacts and verified during TEE bootstrap to prove the running image matches expectations.

---

## Testing Readiness

### Ready for Testing

| Component             | Status | Notes                             |
| --------------------- | ------ | --------------------------------- |
| Client Library (Rust) | Ready  | Stable API, comprehensive tests   |
| Sequencer             | Ready  | Deployed to staging, handles load |
| GCS Transport         | Ready  | Primary storage layer             |
| CBOR/COSE Format      | Ready  | Wire format finalized             |
| Bridge Contract       | Ready  | Deployed to Base Sepolia          |
| CI/CD Pipeline        | Ready  | Reproducible builds, auto-upload  |

**Note**: Not yet audited. Contracts and infrastructure require security review before mainnet deployment.

### Pending for Production (P1)

| Item                     | Status      | Required For             |
| ------------------------ | ----------- | ------------------------ |
| Attestation Verification | Debugging   | RISC Zero proofs working |
| TEE Bootstrap            | Debugging   | Keys registered on-chain |
| Persistent queue         | Not started | Crash recovery           |
| Prometheus metrics       | Partial     | Observability            |
| GCS pagination           | Not started | >1000 batches            |
| State commitments        | Not started | Validator coordination   |

### Deprioritized (P2)

| Item                 | Status             | Notes                             |
| -------------------- | ------------------ | --------------------------------- |
| Arweave publisher    | Documentation only | Add when permanent storage needed |
| Celestia/EigenDA     | Not started        | Alternative DA layers             |
| Validator extensions | Not started        | Custom business logic             |
| Key rotation         | Not started        | Operational improvement           |

### Known Limitations

1. **Python FFI**: The Python FFI bindings open their own SQLite connection via ctypes. Python's native `sqlite3` module creates a separate connection, so changes made through it are NOT captured by SyndDB hooks. For full end-to-end changeset capture, ensure all SQL goes through the FFI connection.

2. **Single Sequencer**: Current architecture assumes single sequencer instance. No leader election or failover.

3. **GCP Confidential Space Only**: TEE attestation flow is GCP-specific. No support for AWS Nitro or Azure Confidential Computing. Applications can run outside TEEs but it is not recommended.

---

## Review Guide: Where to Start

### Recommended Learning Path

```
1. Examples (understand use cases)
       ↓
2. Client Library (how apps integrate)
       ↓
3. Sequencer/Validator (infrastructure)
       ↓
4. Contracts/Bridge (on-chain settlement)
       ↓
5. TEE Bootstrap (attestation flow)
```

### 1. Start with Examples

**Price Oracle** (`examples/price-oracle/`):

- Python application fetching crypto prices
- Custom validator with price consistency rules
- Demonstrates bidirectional message passing
- See `examples/price-oracle/README.md` for setup instructions

**Prediction Market** (`examples/prediction-market/`):

- Rust application with full SyndDB integration
- End-to-end changeset flow
- Better for understanding client library internals
- Prediction market logic is very basic, but it gets the point across

### 2. Client Library Deep Dive

Key files to read in order:

1. **`crates/synddb-client/README.md`** - Usage patterns and API
2. **`crates/synddb-client/src/lib.rs`** - Main `SyndDB` struct and `attach()` function
3. **`crates/synddb-client/src/session.rs`** - SQLite Session Extension integration
4. **`crates/synddb-client/src/sender.rs`** - Background thread for HTTP sends

Key concepts:

- `'static` lifetime requirement for SQLite connection
- Session Extension callback registration
- Changeset batching and compression

### 3. Sequencer and Validator

**Sequencer** (`crates/synddb-sequencer/`):

1. **`src/main.rs`** - Startup, configuration, shutdown
2. **`src/http_api.rs`** - REST endpoints
3. **`src/batcher.rs`** - Batching logic with size/time triggers
4. **`src/transport/gcs.rs`** - GCS publishing

**Validator** (`crates/synddb-validator/`):

1. **`src/main.rs`** - Startup and sync loop
2. **`src/sync/`** - Fetching from sequencer/GCS
3. **`src/apply/`** - Changeset application to SQLite
4. **`src/bridge/`** - Signing for on-chain settlement

### 4. Smart Contracts and Bridge

1. **`contracts/src/Bridge.sol`** - Core message execution
2. **`contracts/src/attestation/AttestationVerifier.sol`** - RISC Zero proof verification
3. **`contracts/src/attestation/TeeKeyManager.sol`** - Key registration

Deployment scripts in `contracts/script/`:

- `DeployBridge.s.sol` - Production deployment
- `DeployLocalDevEnv.s.sol` - Local Anvil deployment

### 5. TEE Bootstrap Flow

Understand how TEE services register their keys on-chain:

1. **`crates/synddb-bootstrap/`** - Key generation and proof request
2. **`crates/gcp-attestation/`** - OIDC token parsing
3. **`crates/proof-service/`** - RISC Zero proof generation
4. **`crates/synddb-relayer/`** - Submit proof to bridge

```
TEE Service Starts
       ↓
Generate secp256k1 keypair
       ↓
Request GCP OIDC attestation token
       ↓
Send to proof-service
       ↓
RISC Zero generates proof
       ↓
Relayer submits to TeeKeyManager
       ↓
Key registered on-chain
```

---

## Development Workflow

### Testing TEE Features

**Preferred approach**: CI + Terraform deployment to GCP Confidential Space. The `just` tooling is useful for individual component testing but cannot test end-to-end TEE flows locally.

```bash
# Push to branch, CI handles:
# - Build reproducible Docker images
# - Deploy to staging (Confidential Space VMs)
# - Run E2E tests in real TEE environment

# Monitor deployment
gh run watch <run-id>
```

### Local Development

For individual component testing (not end-to-end TEE):

```bash
# Run all CI checks
just check

# Run tests
just test

# Start local services (without TEE)
just dev

# See all commands
just
```

### CI/CD

Push to branch triggers:

- Format, lint, unused deps checks
- All tests
- Reproducible Docker builds
- Deploy to staging (GCP Confidential Space)

Check workflow status:

```bash
gh run list --limit 5
gh run watch <run-id>
gh pr checks
```

### Key Documentation Files

| File                | Purpose                             |
| ------------------- | ----------------------------------- |
| `SPEC.md`           | Complete technical specification    |
| `PLAN_SEQUENCER.md` | Sequencer implementation details    |
| `PLAN_VALIDATOR.md` | Validator implementation details    |
| `PLAN_BRIDGE.md`    | Bridge contract specification       |
| `PLAN_TODO.md`      | Unimplemented features and TODOs    |
| `CLAUDE.md`         | AI assistant development guidelines |

---

## Git History: Development Progression

The `example-app` branch contains 568 commits spanning the full development arc. Here's the chronological progression by component area:

### Phase 1: Specification and Planning (Commits 1-15)

**Key commits**: `b0947dd` - `0000108`

- Initial spec (`SPEC.md`) defining the SQLite replication model
- Implementation plans for each component
- Terminology refinement (Core → Sequencer → Sidecar model)
- Workspace setup with Cargo.toml structure

### Phase 2: Core Client Library (Commits 16-40)

**Key commits**: `82a622b` - `87f2665`

- Phase 2 implementation: SQLite Session Extension integration
- Changeset capture via update hooks
- Background sender thread with retry logic
- `.env` file support for configuration
- Prepared statement cache optimization (later removed for simplicity)
- FFI bindings foundation

### Phase 3: Sequencer Implementation (Commits 40-65)

**Key commits**: `17b3131` - `6f11519`

- HTTP receiver for changesets/snapshots
- Batcher with size/time-based flushing
- Chain monitor for blockchain events
- Message passing system (inbound queue, outbound monitor)
- GCS transport layer
- CBOR/COSE wire format

### Phase 4: Validator Implementation (Commits 65-100)

**Key commits**: `4269b68` - `9acdebb`

- Storage fetchers (HTTP, GCS)
- Signature verification
- Changeset application to SQLite replica
- Audit trail verification
- Pending changeset chain validation
- Schema mismatch detection

### Phase 5: Smart Contracts (Commits 100-130)

**Key commits**: `ffe884a` - `d0c4f01`

- Bridge.sol with message execution
- ModuleCheckRegistry for pluggable validation
- TeeKeyManager for key registration
- GasTreasury for gas sponsorship
- Deployed to Base Sepolia

### Phase 6: Example Applications (Commits 130-180)

**Key commits**: `44378c2` - `690ef92`

- Prediction market (Rust): Full SyndDB integration demo
- Price oracle (Python): Custom validator rules, message passing
- PriceOracle.sol contract
- Development environment scripts
- Docker E2E tests

### Phase 7: TEE Integration (Commits 180-280)

**Key commits**: `a789778` - `cb19c2e`

- GCP Confidential Space attestation sample
- JWT signature verification
- `gcp-attestation` crate for token parsing
- `synddb-bootstrap` crate for key registration
- Proof-service for SP1/RISC Zero proving
- Cosign image signing (later migrated to secp256k1)

### Phase 8: Infrastructure and Deployment (Commits 280-400)

**Key commits**: `11b040f` - `7cb896e`

- Terraform modules for GCP deployment
- Reproducible Docker builds
- Artifact Registry CI/CD pipeline
- Relayer service for gas sponsorship
- SLSA provenance attestations
- Cost-optimized CI workflows

### Phase 9: Production Hardening (Commits 400-500)

**Key commits**: `ed76f9b` - `110ec75`

- secp256k1 image signing (replacing cosign P-256)
- OCI artifact signatures
- RISC Zero migration (from SP1)
- Prometheus metrics foundation
- OpenTelemetry/Cloud Trace integration
- Terraform auto-updates for image digests

### Phase 10: Current Work (Commits 500-568)

**Key commits**: `636a436` - `052a03e`

- Client library test helpers
- RISC Zero 3.0 toolchain fixes
- CI workflow refinements
- Documentation updates
- Bug fixes for TEE bootstrap flow

---

## Commit Statistics by Area

| Area              | Approximate Commits | Key Milestones                                     |
| ----------------- | ------------------- | -------------------------------------------------- |
| Client Library    | ~80                 | Session Extension, FFI bindings, push/snapshot API |
| Sequencer         | ~70                 | HTTP API, batcher, message passing, GCS transport  |
| Validator         | ~60                 | Sync/apply, audit trail, schema detection          |
| Contracts         | ~50                 | Bridge, TeeKeyManager, attestation verification    |
| Examples          | ~40                 | Prediction market, price oracle                    |
| TEE/Attestation   | ~80                 | Bootstrap, proof-service, RISC Zero migration      |
| Infrastructure    | ~100                | Terraform, CI/CD, reproducible builds              |
| Documentation     | ~40                 | SPEC, PLAN files, READMEs                          |
| Refactoring/Style | ~50                 | Terminology, clippy, formatting                    |

---

## Contact and Resources

- **Repository**: `SyndicateProtocol/synddb`
- **Staging Environment**: GCP project `synddb-staging`
- **Base Sepolia Contract**: `0x7ddE947a7DF980fD00887371C527cBbcB4E92a4E`
