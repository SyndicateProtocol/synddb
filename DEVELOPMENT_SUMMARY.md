# SyndDB Development Summary

This document provides an overview of SyndDB's development progression, key architectural decisions, and production readiness status for team members reviewing the codebase.

## Executive Summary

SyndDB is a SQLite replication system that enables high-performance blockchain applications. Applications write to SQLite normally while a client library captures changes and sends them to a sequencer service, which publishes to data availability layers. Validators verify the SQL operations and sign for on-chain settlement.

CLAUDE: Mention that the current goal is to complete TEE bootstrapping, which is necessary for allowing sequencers + validators to write to the Bridge
**Current Status**: Core infrastructure complete. Staging environment deployed on GCP Confidential Space with Base Sepolia integration. Examples demonstrate end-to-end functionality.

---

## Component Overview

CLAUDE: Mention somewhere in this section that we're still prototyping changesets vs WAL parsing. Also mention briefly what changesets and WAL parsing are for people unfamiliar with SQLite terminology.

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

**Rationale**: RISC Zero's network prover provides cost-effective proof generation without requiring GPU infrastructure. The zkVM proves that a GCP Confidential Space attestation token is valid, allowing the bridge to trust TEE-signed messages.

CLAUDE: Mention that this is because SP1 has a closed source dependency that cannot run in Google Cloud Run, while RISC Zero likely can be run in Cloud Run and has an open source GPU prover.
**Evolution**: Started with SP1 SDK, migrated to RISC Zero 3.0 for better prover economics.

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

CLAUDE: Refer to this as "Testing readiness" rather than "Production readiness" since we're not live yet or audited.

## Production Readiness

### Ready for Production

| Component                | Status     | Notes                             |
| ------------------------ | ---------- | --------------------------------- |
| Client Library (Rust)    | Production | Stable API, comprehensive tests   |
| Sequencer                | Production | Deployed to staging, handles load |
| GCS Transport            | Production | Primary storage layer             |
| CBOR/COSE Format         | Production | Wire format finalized             |
| Bridge Contract          | Production | Deployed to Base Sepolia          |
| Attestation Verification | Production | RISC Zero proofs working          |
| TEE Bootstrap            | Production | Keys registered on-chain          |
| CI/CD Pipeline           | Production | Reproducible builds, auto-deploy  |

### Pending for Production (P1)

| Item                      | Status          | Required For             |
| ------------------------- | --------------- | ------------------------ |
| Validator TEE integration | Documented only | Decentralized validation |
| Persistent queue          | Not started     | Crash recovery           |
| Prometheus metrics        | Partial         | Observability            |
| GCS pagination            | Not started     | >1000 batches            |
| State commitments         | Not started     | Validator coordination   |

### Deprioritized (P2)

| Item                 | Status             | Notes                             |
| -------------------- | ------------------ | --------------------------------- |
| Arweave publisher    | Documentation only | Add when permanent storage needed |
| Celestia/EigenDA     | Not started        | Alternative DA layers             |
| Validator extensions | Not started        | Custom business logic             |
| Key rotation         | Not started        | Operational improvement           |

### Known Limitations

CLAUDE: Is this correct? Are you sure? That's a big bug.

1. **Python FFI**: Creates separate SQLite connection, changesets not captured. Use Rust client for full integration.

2. **Single Sequencer**: Current architecture assumes single sequencer instance. No leader election or failover.

CLAUDE: We're fixing this with automatic registration based on the TEE version. An admin can choose whether to have automatic approval of validators or not.

3. **Validator Signatures**: Bridge accepts threshold of validator signatures, but validator registration/coordination is manual.

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

```bash
just examples::price-oracle
```

**Prediction Market** (`examples/prediction-market/`):

- Rust application with full SyndDB integration
- End-to-end changeset flow
- Better for understanding client library internals

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

CLAUDE: Mention that CI + Terraform is preferred to test the TEE environment. The just tooling is only useful for specific components, but not end-to-end workflows.

### Running Locally

```bash
# Full local environment
just dev

# Run all CI checks
just check

# Run tests
just test

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

CLAUDE: Summarize the commits in example-app. You're missing A LOT of them. Go step by step and summarize the work chronologically. In particular, explain how these batches tie to specific components. e.g. When we fixed client bugs, when we fixed the sequencer, when we added Terraform configs, etc.

## Git History Highlights

The development progressed through these major phases:

### Phase 1: Foundation (Commits 1-20)

- Initial spec and implementation plans
- Core client library with Session Extension
- Basic sequencer with local storage

### Phase 2: Infrastructure (Commits 20-50)

- GCS transport layer
- CBOR/COSE wire format
- Message passing system
- Chain monitor for blockchain events

### Phase 3: TEE Integration (Commits 50-70)

- GCP Confidential Space support
- Attestation token parsing
- SP1 zkVM proofs (later migrated to RISC Zero)
- Reproducible Docker builds

### Phase 4: Production Hardening (Commits 70-85)

- secp256k1 image signing
- Terraform infrastructure
- RISC Zero 3.0 migration
- CI/CD automation
- Staging deployment

---

## Contact and Resources

- **Repository**: `SyndicateProtocol/synddb`
- **Staging Environment**: GCP project `synddb-staging`
- **Base Sepolia Contract**: `0x7ddE947a7DF980fD00887371C527cBbcB4E92a4E`
