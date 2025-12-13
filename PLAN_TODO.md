# PLAN_TODO.md - Unimplemented Items

This document tracks major unimplemented features identified from the PLAN and SPEC files compared to the current codebase.

## Priority Legend

- **P0**: Critical for production
- **P1**: Important for production readiness
- **P2**: Nice to have / can defer

---

## 1. Sequencer DA Layer Publishers [P0]

**Source**: `PLAN_SEQUENCER.md`

**Status**: Stub implementations only - Local and GCS are implemented

| Publisher | File | Status |
|-----------|------|--------|
| Celestia | `crates/synddb-sequencer/src/publish/celestia.rs` | Stub - TODO: Use celestia-client |
| EigenDA | `crates/synddb-sequencer/src/publish/eigenda.rs` | Stub - TODO: Use eigenda-rust |
| IPFS | `crates/synddb-sequencer/src/publish/ipfs.rs` | Stub - TODO: Use ipfs-api |
| Arweave | `crates/synddb-sequencer/src/publish/arweave.rs` | Stub - TODO: Use arweave-rs |

**Implementation Notes**:
- Each publisher should implement the `DAPublisher` trait from `crates/synddb-sequencer/src/publish/traits.rs`
- Add corresponding feature flags to `Cargo.toml`
- Wire up in `main.rs` with feature gates

---

## 2. Message Passing System [P0]

**Source**: `PLAN_MESSAGE_PASSING.md`

**Status**: Not implemented

The sequencer needs bidirectional message passing between applications and blockchain. Current `http_api.rs` only handles changesets/snapshots/withdrawals.

### 2.1 Inbound Message Monitor

Monitor blockchain for bridge events and deliver to applications.

**Components needed**:
- `InboundMonitor` - WebSocket subscription to Bridge contract events
- `MessageQueue` - In-memory queue with retention and size limits
- SSE endpoint for real-time delivery: `GET /messages/inbound/stream`
- REST polling endpoint: `GET /messages/inbound?after_id={id}`
- Acknowledgment endpoint: `POST /messages/inbound/{id}/ack`

### 2.2 Outbound Message Monitor

Monitor SQLite message tables and publish to DA layers.

**Components needed**:
- `OutboundMonitor` - Poll `message_log` and `inbound_message_log` tables (read-only)
- Validate message context (trigger references exist)
- Status endpoint: `GET /messages/outbound/{id}/status`

### 2.3 Consistency Enforcer

Ensure all inbound messages are acknowledged before processing outbound.

**Components needed**:
- `ConsistencyEnforcer` - Track blockchain messages vs acknowledged messages
- Halt outbound processing when consistency violated
- `can_process_outbound()` check before publishing

### 2.4 Progressive Degradation Manager

4-level degradation strategy for system health.

| Level | Duration | Missing Msgs | Status | Actions |
|-------|----------|--------------|--------|---------|
| L1: Warning | < 30s | < 3 | Healthy | Alert app, increase retry |
| L2: Degraded | < 5min | < 10 | Degraded | Halt outbound, restrict API |
| L3: Critical | < 30min | < 50 | Critical | Fail health checks |
| L4: Halt | > 30min | > 50 | Halted | Full shutdown |

### 2.5 State Commitments

Signed commitments for validators about system state.

**Components needed**:
- `StateCommitment` struct with sequence, status, error code, state hash
- `StateCommitmentPublisher` - Sign with TEE key and publish to DA
- Publish even when halted for validator visibility

---

## 3. Sequencer Operational Features [P1]

**Source**: `PLAN_SEQUENCER.md` (marked as TODO sections)

### 3.1 Persistent Queue

**Problem**: Sequencer crash loses in-memory changesets

**Solution**: Disk buffer before DA publish
```
src/publish/persistent_queue.rs
- enqueue() - Write to disk before publishing
- dequeue() - Mark as published after DA confirmation
- recover() - On restart, republish unconfirmed payloads
```

### 3.2 Backpressure Handling

**Problem**: Application writes faster than sequencer can publish

**Solution**: Bounded channels with monitoring
- Bounded `mpsc::channel` for changesets (e.g., 1000 capacity)
- Monitor queue depth, alert when falling behind
- Metrics for queue depth

### 3.3 Graceful Shutdown

**Problem**: Shutdown could lose in-flight changesets

**Solution**: Flush all pending work before exit
1. Stop accepting new changesets
2. Flush all pending batches
3. Wait for attestor to finish
4. Wait for all DA publishes to confirm
5. Persist state checkpoint

### 3.4 Large Transaction Handling

**Problem**: Single transaction with millions of rows creates huge changeset

**Solution**: Detect and force snapshot
- `MAX_CHANGESET_SIZE` threshold (e.g., 100MB)
- If exceeded, discard changesets and create snapshot instead

### 3.5 Key Rotation

**Problem**: Ethereum signing keys may need rotation

**Solution**: Key rotation protocol
- Generate new key in TEE
- Seal to Secret Manager
- Publish `KeyRotation` message to validators
- Keep old key active during transition period

### 3.6 Observability/Metrics

**Problem**: Insufficient monitoring

**Solution**: Comprehensive metrics
- `changeset_lag_seconds` - Time from commit to publish
- `queue_depth` - Unpublished changesets
- `changesets_per_second` / `bytes_published_per_second`
- `da_publish_failures` / `schema_detection_errors`
- Health status per DA layer

### 3.7 Batching Layer

**Source**: `crates/synddb-sequencer/src/publish/traits.rs:9`

**Status**: TODO - documented but not implemented

**Problem**: Messages published one at a time, inefficient for high throughput

**Solution**: Implement batching layer between HTTP handlers and StoragePublisher
```rust
pub struct BatchConfig {
    /// Maximum messages per batch before flushing (default: 50)
    pub batch_size: usize,
    /// Maximum time to wait before flushing a partial batch (default: 5s)
    pub batch_interval: Duration,
    /// Enable zstd compression for batches (recommended for batch_size > 1)
    pub compress: bool,
}
```

Two possible modes:
- Fire-and-forget: HTTP handler returns immediately after sequencing
- Wait-for-batch: HTTP handler waits until its message's batch is published

---

## 4. Validator TEE Integration [P1]

**Source**: `PLAN_VALIDATOR.md`

**Status**: Documented but not implemented

### 4.1 ConfidentialValidator

GCP Confidential Space integration for validators.

**Components needed**:
```
src/validator/confidential_validator.rs
- init() - Load or generate validator keypair
- generate_attestation() - Fetch token from GCP metadata service
- sign_message() - Sign with TEE-protected key
- sign_state_update() - Sign state updates with attestation
- refresh_attestation_if_needed() - Hourly refresh
```

### 4.2 Validator Key Management

- Generate secp256k1 keypair on init
- Store in GCP Secret Manager with Workload Identity binding
- Keys bound to container measurements (image digest)

### 4.3 Bridge Registration

- Generate zkVM proof for attestation (SP1)
- Call `Bridge.registerValidator()` with attestation + proof
- Store registered tx hash

### 4.4 State Update Signatures

```rust
struct StateUpdateSignature {
    state_update_hash: H256,
    sequence: u64,
    signature: Vec<u8>,
    validator: Address,
    attestation_token: String,
}
```

---

## 5. Validator Metrics [P1]

**Source**: `crates/synddb-validator/src/metrics.rs`

**Status**: Empty file with TODO comment listing planned metrics

**Planned Prometheus metrics**:
- `synddb_validator_messages_synced_total` - Counter of synced messages
- `synddb_validator_last_sequence` - Gauge of last synced sequence number
- `synddb_validator_sync_lag_seconds` - Gauge of time since last sync
- `synddb_validator_changeset_apply_duration_seconds` - Histogram of apply times
- `synddb_validator_signature_verify_duration_seconds` - Histogram of verify times
- `synddb_validator_gaps_detected_total` - Counter of detected gaps
- `synddb_validator_withdrawals_signed_total` - Counter of signed withdrawals

---

## 6. Validator Extension System [P2]

**Source**: `PLAN_VALIDATOR.md`

**Status**: Planned but not implemented

### 6.1 WithdrawalValidator Trait

```rust
#[async_trait]
pub trait WithdrawalValidator: Send + Sync {
    async fn validate(&self, withdrawal: &PendingWithdrawal) -> Result<()>;
}
```

### 6.2 Example Extensions

- `WithdrawalRateLimiter` - Daily withdrawal limits per address
- Custom business logic validators
- External API verification

### 6.3 Validator HTTP Protocols

**Source**: `PLAN_VALIDATOR.md:206`

Future protocols beyond REST:
- JSON-RPC support
- WebSocket subscriptions for real-time updates

---

## 7. Chain Monitor / Bridge Integration [P1]

**Source**: `PLAN_BRIDGE.md`, `crates/synddb-chain-monitor/`

**Status**: Partially implemented with TODOs

### 7.1 Bridge Event Definitions

- `crates/synddb-chain-monitor/src/events.rs:9` - "TODO: Update with finalized Bridge contract events"
- `crates/synddb-chain-monitor/src/events.rs:10` - "TODO: Test what happens if Bridge interacts with multiple contracts"
- Define event types matching `contracts/src/Bridge.sol` events:
  - `MessageInitialized`
  - `MessageHandled`
  - `NativeTokenWrapped`
  - `NativeTokenUnwrapped`

### 7.2 Bridge Deployment Metadata

- `crates/synddb-chain-monitor/src/config.rs:25` - "TODO - automatically get this from Bridge deployment metadata"
- Auto-discover Bridge contract address from deployment artifacts

---

## 8. Additional Validator Fetchers [P2]

**Source**: `PLAN_VALIDATOR.md`

**Status**: Only HTTP and GCS implemented

| Fetcher | Status |
|---------|--------|
| Celestia | Planned - needs celestia-client integration |
| EigenDA | Planned - needs eigenda-rust integration |
| IPFS | Planned - needs ipfs-api integration |
| Arweave | Planned - needs arweave-rs integration |

**Implementation Notes**:
- Each fetcher implements `StorageFetcher` trait from `crates/synddb-validator/src/sync/providers/mod.rs`
- Mirror the publisher implementations from sequencer

---

## 9. Client Language Bindings [P2]

**Source**: `crates/synddb-client/examples/`

**Status**: Go bindings exist with TODOs, Python not implemented

### 9.1 Python Bindings

- `crates/synddb-client/examples/README.md:145` - "Status: TODO - Native Python bindings not yet implemented"
- `crates/synddb-client/examples/python_example.py:5` - "WORK IN PROGRESS - NOT FUNCTIONAL"
- Options: PyO3 bindings or C FFI wrapper

### 9.2 Go Bindings

- `crates/synddb-client/bindings/go/synddb.go:58` - "TODO: Platform-specific extraction of sqlite3*"
- Complete CGO integration for SQLite pointer handling
- Currently passes null pointer - needs driver-specific extraction

---

## 10. GCS Provider Improvements [P2]

**Source**: `crates/synddb-sequencer/src/publish/gcs.rs`, `crates/synddb-validator/src/sync/providers/gcs.rs`

### 10.1 Batch Listing Pagination

- `crates/synddb-sequencer/src/publish/gcs.rs:243` - "TODO: This is O(n) in the number of batches"
- `crates/synddb-validator/src/sync/providers/gcs.rs:187` - "TODO: GCS returns max 1000 objects per request"
- Need to implement pagination for large batch counts

### 10.2 GCS Architecture Review

- `crates/synddb-validator/src/sync/providers/gcs.rs:17` - "TODO revisit this"

---

## 11. Batch File Optimization [P2]

**Source**: `docs/plans/batch-size-optimization.md`

**Status**: Base64 encoding implemented, compression and COSE planned

### 11.1 Compress Batch Files

**Effort**: Low | **Benefit**: ~2-3x for single messages, ~5-10x for multi-message batches

- Store as `.json.zst` or `.json.gz`
- GCS can serve with automatic decompression via `Content-Encoding`

### 11.2 COSE Format

**Effort**: High | **Benefit**: Optimal compactness + standardization

Replace JSON with COSE (CBOR Object Signing and Encryption, RFC 8152):
- No encoding overhead for payloads (raw binary)
- Standard format for signed data
- Rust libraries: `coset`, `cose-rust`

Migration approach:
1. Implement COSE writer in sequencer
2. Implement COSE reader in validator
3. Support both formats during transition
4. Eventually deprecate JSON format

---

## 12. Release Process Improvements [P2]

**Source**: `RELEASING.md:275`

**Status**: Manual release process, automation planned

- [ ] Publish to npm (@synddb/client-native)
- [ ] Publish to PyPI (synddb-client)
- [ ] Automated changelog generation
- [ ] Automated version bumping
- [ ] Cross-compilation for more platforms (ARM Linux, etc.)
- [ ] Signed binaries
- [ ] Notarization for macOS binaries

---

## 13. E2E Test Improvements [P2]

**Source**: `tests/e2e/runner/src/runner.rs:54`

### 13.1 DA Test Configuration

- `tests/e2e/runner/src/runner.rs:54` - "TODO CLAUDE fix this"
- DA API tests currently skipped for external DA layers (GCS)
- Need proper configuration for different DA backends

---

## 14. Ignored/Skipped Tests [P2]

Tests marked with `#[ignore]` that need environments or manual verification:

| Test | Location | Reason |
|------|----------|--------|
| Attestation network test | `crates/synddb-sequencer/src/attestation.rs:351` | Requires network access |
| GCP Confidential Space test | `crates/synddb-client/src/attestation.rs:284` | Requires GCP environment |

---

## Implementation Checklist

### Phase 1: Core Production Features [P0]

- [ ] Celestia publisher implementation
- [ ] EigenDA publisher implementation
- [ ] Inbound message monitor
- [ ] Outbound message monitor
- [ ] Consistency enforcer
- [ ] Message passing HTTP API

### Phase 2: Production Readiness [P1]

- [ ] Persistent queue for crash recovery
- [ ] Graceful shutdown
- [ ] Validator TEE integration (ConfidentialValidator)
- [ ] Bridge event definitions (update from sample events)
- [ ] Progressive degradation manager
- [ ] State commitments
- [ ] Validator Prometheus metrics
- [ ] Batching layer for publishers
- [ ] Bridge deployment metadata auto-discovery

### Phase 3: Extended Features [P2]

- [ ] IPFS publisher
- [ ] Arweave publisher
- [ ] Validator extension system
- [ ] Python bindings (PyO3)
- [ ] Complete Go bindings (CGO sqlite3*)
- [ ] Key rotation protocol
- [ ] Additional validator fetchers
- [ ] GCS pagination for large batch counts
- [ ] Batch file compression (.json.zst)
- [ ] COSE format for batches
- [ ] Release automation (npm, PyPI, changelog)
- [ ] E2E test DA configuration fix
- [ ] Validator JSON-RPC/WebSocket protocols

---

## Code TODOs Summary

Quick reference of all TODO comments found in the codebase:

| File | Line | Description |
|------|------|-------------|
| `synddb-sequencer/src/publish/celestia.rs` | 21-22 | Use celestia-client, return blob ID |
| `synddb-sequencer/src/publish/eigenda.rs` | 21-22 | Use eigenda-rust, return blob reference |
| `synddb-sequencer/src/publish/ipfs.rs` | 21-22 | Use ipfs-api, return CID |
| `synddb-sequencer/src/publish/arweave.rs` | 21-22 | Use arweave-rs, return tx ID |
| `synddb-sequencer/src/publish/traits.rs` | 9 | Implement batching layer |
| `synddb-sequencer/src/publish/gcs.rs` | 243 | O(n) batch listing optimization |
| `synddb-validator/src/metrics.rs` | 3 | Implement Prometheus metrics |
| `synddb-validator/src/sync/providers/gcs.rs` | 17 | Revisit GCS architecture |
| `synddb-validator/src/sync/providers/gcs.rs` | 187 | GCS 1000 object pagination |
| `synddb-chain-monitor/src/events.rs` | 9-10 | Update Bridge events, test multi-contract |
| `synddb-chain-monitor/src/config.rs` | 25 | Auto-get Bridge deployment metadata |
| `synddb-client/bindings/go/synddb.go` | 58 | Platform-specific sqlite3* extraction |
| `synddb-client/examples/python_example.py` | 21 | Implement Python bindings |
| `tests/e2e/runner/src/runner.rs` | 54 | Fix DA test configuration |

---

## References

- `SPEC.md` - Overall system specification
- `PLAN_SEQUENCER.md` - Sequencer implementation plan
- `PLAN_VALIDATOR.md` - Validator implementation plan
- `PLAN_BRIDGE.md` - Bridge contract specification
- `PLAN_MESSAGE_PASSING.md` - Message passing system design
- `docs/plans/batch-size-optimization.md` - Batch optimization strategies
- `RELEASING.md` - Release process documentation
