# PLAN_TODO.md - Unimplemented Items

This document tracks major unimplemented features identified from the PLAN and SPEC files compared to the current codebase.

## Priority Legend

- **P0**: Critical for production
- **P1**: Important for production readiness
- **P2**: Nice to have / can defer

---

## 1. Sequencer DA Layer Publishers [P2 - Deprioritized]

**Source**: `PLAN_SEQUENCER.md`

**Status**: GCS is the primary DA layer. Additional publishers are deprioritized.

| Publisher | File | Status | Priority |
|-----------|------|--------|----------|
| GCS | `crates/synddb-sequencer/src/transport/gcs.rs` | ✅ Implemented | Primary |
| Local | `crates/synddb-sequencer/src/transport/local.rs` | ✅ Implemented | Dev/Testing |
| Arweave | `crates/synddb-sequencer/src/transport/arweave.rs` | Documentation only | Next if needed |
| Celestia | N/A | Not started | Deprioritized |
| EigenDA | N/A | Not started | Deprioritized |
| IPFS | N/A | Not started | Deprioritized |

**Decision**: Focus on GCS for production. Arweave would be next if permanent storage is required.

**Implementation Notes** (for Arweave if needed):
- Implement `TransportPublisher` trait from `crates/synddb-sequencer/src/transport/traits.rs`
- Detailed implementation plan exists in `transport/arweave.rs` module docs
- Add `arweave` feature flag to `Cargo.toml`

---

## 2. Message Passing System [Largely Complete]

**Source**: `PLAN_MESSAGE_PASSING.md`

**Status**: Core components implemented. Some advanced features pending.

### 2.1 Inbound Message Monitor ✅ Partially Complete

**Implemented** via `synddb-chain-monitor` crate:
- ✅ `ChainMonitor` - WebSocket subscription to contract events
- ✅ `EventStore` - SQLite-backed persistent event storage
- ✅ Event parsing and handler dispatch

**Implemented** via `synddb-sequencer/src/messages/`:
- ✅ `MessageQueue` - In-memory queue with retention and size limits
- ✅ REST polling endpoint: `GET /messages/inbound?after_id={id}`
- ✅ Acknowledgment endpoint: `POST /messages/inbound/{id}/ack`
- ✅ Push endpoint for chain monitors: `POST /messages/inbound`

**Not implemented**:
- SSE endpoint for real-time delivery (optional optimization)

### 2.2 Outbound Message Monitor ✅ Complete

**Implemented** in `crates/synddb-sequencer/src/messages/outbound.rs`:
- ✅ `OutboundMonitor` - Polls `message_log` table (read-only)
- ✅ `OutboundTracker` - In-memory state tracking
- ✅ Status endpoint: `GET /messages/outbound/{id}/status`
- ✅ Stats endpoint: `GET /messages/outbound/stats`

### 2.3 Consistency Enforcer [P2]

Not implemented. Lower priority - can be added when needed.

### 2.4 Progressive Degradation Manager [P2]

Not implemented. Lower priority - can be added when needed.

### 2.5 State Commitments [P1]

Not implemented. Would be useful for validator visibility into sequencer state.

---

## 3. Sequencer Operational Features [Partially Complete]

**Source**: `PLAN_SEQUENCER.md`

### 3.1 Persistent Queue [P1]

**Status**: Not implemented

**Problem**: Sequencer crash loses in-memory changesets

**Solution**: Disk buffer before DA publish
```
src/publish/persistent_queue.rs
- enqueue() - Write to disk before publishing
- dequeue() - Mark as published after DA confirmation
- recover() - On restart, republish unconfirmed payloads
```

### 3.2 Backpressure Handling ✅ Complete

**Implemented** via bounded channels in `Batcher`:
- Bounded `mpsc::channel` for batcher commands
- Stats tracking for pending messages/bytes

### 3.3 Graceful Shutdown ✅ Complete

**Implemented** in `crates/synddb-sequencer/src/main.rs`:
- ✅ Ctrl+C / SIGTERM signal handling
- ✅ Batcher flush on shutdown
- ✅ Configurable shutdown timeout
- ✅ Watch channel for shutdown coordination

### 3.4 Large Transaction Handling [P2]

**Status**: Not implemented

**Problem**: Single transaction with millions of rows creates huge changeset

**Solution**: Detect and force snapshot
- `MAX_CHANGESET_SIZE` threshold (e.g., 100MB)
- If exceeded, discard changesets and create snapshot instead

### 3.5 Key Rotation [P2]

**Status**: Not implemented

**Problem**: Ethereum signing keys may need rotation

**Solution**: Key rotation protocol
- Generate new key in TEE
- Seal to Secret Manager
- Publish `KeyRotation` message to validators
- Keep old key active during transition period

### 3.6 Observability/Metrics [P1]

**Status**: Partial - BatchStats exists, Prometheus endpoint not implemented

**Implemented**:
- ✅ `BatchStats` struct with counters for batches/messages/bytes
- ✅ Compression ratio tracking

**Not implemented**:
- Prometheus `/metrics` endpoint
- `changeset_lag_seconds`, `da_publish_failures` metrics

### 3.7 Batching Layer ✅ Complete

**Implemented** in `crates/synddb-sequencer/src/batcher.rs`:
- ✅ `Batcher` struct with configurable batch size, interval, and byte limits
- ✅ `BatchConfig` with `max_messages`, `max_batch_bytes`, `flush_interval`
- ✅ Async flush on interval, size, or shutdown
- ✅ CBOR+zstd compression
- ✅ Fire-and-forget mode (HTTP returns after sequencing)

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

## 8. Additional Validator Fetchers [P2 - Deprioritized]

**Source**: `PLAN_VALIDATOR.md`

**Status**: HTTP and GCS implemented. Additional fetchers deprioritized (matching sequencer DA decision).

| Fetcher | Status | Priority |
|---------|--------|----------|
| HTTP | ✅ Implemented | Primary (sequencer API) |
| GCS | ✅ Implemented | Primary (batch storage) |
| Arweave | Not started | Next if needed |
| Celestia | Not started | Deprioritized |
| EigenDA | Not started | Deprioritized |
| IPFS | Not started | Deprioritized |

**Implementation Notes**:
- Fetchers implement `StorageFetcher` trait from `crates/synddb-validator/src/sync/fetcher.rs`
- Would mirror sequencer transport implementations

---

## 9. Client Language Bindings [Partially Complete]

**Source**: `crates/synddb-client/bindings/`

### 9.1 Python Bindings ✅ Complete

**Implemented** in `crates/synddb-client/bindings/python/synddb.py`:
- ✅ Pure Python FFI wrapper using `ctypes` (no compilation needed)
- ✅ `SyndDB` class with `attach()`, `execute()`, `snapshot()`, `detach()`
- ✅ `MessageClient` class for message passing API
- ✅ Transaction support (`begin()`, `commit()`, `rollback()`)
- ✅ Context manager support

### 9.2 Go Bindings [P2 - Incomplete]

**Status**: Stub only - passes null pointer for SQLite handle

- `crates/synddb-client/bindings/go/synddb.go:58` - "TODO: Platform-specific extraction of sqlite3*"
- Needs CGO integration for extracting raw `sqlite3*` from `database/sql`
- Would require driver-specific code for `github.com/mattn/go-sqlite3`

### 9.3 Node.js Bindings ✅ Complete

**Implemented** in `crates/synddb-client/bindings/nodejs/`:
- ✅ FFI wrapper using `ffi-napi`
- ✅ Async/await API

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

### Phase 1: Production Readiness [P1]

- [ ] Persistent queue for crash recovery
- [ ] Validator TEE integration (ConfidentialValidator)
- [ ] Bridge event definitions (update from sample events)
- [ ] State commitments
- [ ] Validator Prometheus metrics
- [ ] Sequencer Prometheus metrics endpoint
- [ ] GCS pagination for >1000 batches

### Phase 2: Extended Features [P2]

- [ ] Arweave publisher (if permanent storage needed)
- [ ] Validator extension system (WithdrawalValidator trait)
- [ ] Complete Go bindings (CGO sqlite3*)
- [ ] Key rotation protocol
- [ ] Large transaction handling (force snapshot)
- [ ] Consistency enforcer
- [ ] Progressive degradation manager
- [ ] Release automation (npm, PyPI, changelog)
- [ ] Validator JSON-RPC/WebSocket protocols

---

## Code TODOs Summary

Quick reference of actual TODO comments found in the codebase:

| File | Line | Description | Priority |
|------|------|-------------|----------|
| `synddb-validator/src/metrics.rs` | 3 | Implement Prometheus metrics | P1 |
| `synddb-validator/src/sync/providers/gcs.rs` | 195 | GCS 1000 object pagination | P1 |
| `synddb-validator/src/main.rs` | 188 | Check messages are in order | P2 |
| `synddb-validator/src/validator.rs` | 584 | Handle out-of-sequence errors | P2 |
| `synddb-chain-monitor/src/events.rs` | 9-10 | Update Bridge events to match real contract | P1 |
| `synddb-chain-monitor/src/config.rs` | 30 | Auto-get Bridge deployment metadata | P2 |
| `synddb-sequencer/src/transport/arweave.rs` | 136 | Implement ArweaveTransport | P2 |
| `synddb-sequencer/src/transport/mod.rs` | 23 | Additional DA transports | P2 |
| `synddb-client/bindings/go/synddb.go` | 58 | Platform-specific sqlite3* extraction | P2 |

---

## References

- `SPEC.md` - Overall system specification
- `PLAN_SEQUENCER.md` - Sequencer implementation plan
- `PLAN_VALIDATOR.md` - Validator implementation plan
- `PLAN_BRIDGE.md` - Bridge contract specification
- `PLAN_MESSAGE_PASSING.md` - Message passing system design
- `docs/plans/batch-size-optimization.md` - Batch optimization strategies
- `RELEASING.md` - Release process documentation

---

## Maintenance: How to Update This Document

When refreshing this TODO list, follow these steps:

1. **Read PLAN and SPEC files** - Glob for `**/{PLAN,SPEC}*.md` and compare documented features against implementation

2. **Search for TODO markers** - Grep for common patterns:
   ```
   TODO|FIXME|XXX|HACK|unimplemented!|todo!
   ```

3. **Search for stub/placeholder code** - Grep for:
   ```
   stub|placeholder|not yet|not implemented|WIP
   ```

4. **Check for ignored tests** - Grep for `#[ignore]` to find tests requiring special environments

5. **Search for future work references** - Grep in `*.md` files for:
   ```
   future|roadmap|planned|eventually
   ```

6. **Review feature flags** - Check `Cargo.toml` files for features with stub implementations

7. **Check empty/minimal files** - Look for files with only TODO comments or minimal content (e.g., `metrics.rs`)

8. **Review docs/ directory** - Check `docs/plans/` for optimization or improvement documents

9. **Check RELEASING.md** - Look for "Future Improvements" sections

10. **Verify against crate READMEs** - Each crate's README may list planned features

After updates, organize items by priority (P0/P1/P2) and update the Implementation Checklist.
