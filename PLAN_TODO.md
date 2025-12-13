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

## 5. Validator Extension System [P2]

**Source**: `PLAN_VALIDATOR.md`

**Status**: Planned but not implemented

### 5.1 WithdrawalValidator Trait

```rust
#[async_trait]
pub trait WithdrawalValidator: Send + Sync {
    async fn validate(&self, withdrawal: &PendingWithdrawal) -> Result<()>;
}
```

### 5.2 Example Extensions

- `WithdrawalRateLimiter` - Daily withdrawal limits per address
- Custom business logic validators
- External API verification

---

## 6. Chain Monitor / Bridge Integration [P1]

**Source**: `PLAN_BRIDGE.md`, `crates/synddb-chain-monitor/`

**Status**: Partially implemented with TODOs

### 6.1 Bridge Event Definitions

- `crates/synddb-chain-monitor/src/events.rs:9` - "TODO: Update with finalized Bridge contract events"
- Define event types matching `contracts/src/Bridge.sol` events:
  - `MessageInitialized`
  - `MessageHandled`
  - `NativeTokenWrapped`
  - `NativeTokenUnwrapped`

### 6.2 Bridge Deployment Metadata

- `crates/synddb-chain-monitor/src/config.rs:25` - "TODO - automatically get this from Bridge deployment metadata"
- Auto-discover Bridge contract address from deployment artifacts

---

## 7. Additional Validator Fetchers [P2]

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

## 8. Client Language Bindings [P2]

**Source**: `crates/synddb-client/examples/`

**Status**: Go bindings exist with TODOs, Python not implemented

### 8.1 Python Bindings

- `crates/synddb-client/examples/README.md:145` - "Status: TODO - Native Python bindings not yet implemented"
- Options: PyO3 bindings or C FFI wrapper

### 8.2 Go Bindings

- `crates/synddb-client/bindings/go/synddb.go:58` - "TODO: Platform-specific extraction of sqlite3*"
- Complete CGO integration for SQLite pointer handling

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
- [ ] Bridge event definitions
- [ ] Progressive degradation manager
- [ ] State commitments

### Phase 3: Extended Features [P2]

- [ ] IPFS publisher
- [ ] Arweave publisher
- [ ] Validator extension system
- [ ] Python bindings
- [ ] Complete Go bindings
- [ ] Key rotation protocol
- [ ] Additional validator fetchers

---

## References

- `SPEC.md` - Overall system specification
- `PLAN_SEQUENCER.md` - Sequencer implementation plan
- `PLAN_VALIDATOR.md` - Validator implementation plan
- `PLAN_BRIDGE.md` - Bridge contract specification
- `PLAN_MESSAGE_PASSING.md` - Message passing system design
