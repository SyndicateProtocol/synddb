# Refactoring Plan: Remove JSON + Base64 Legacy Format

**STATUS: COMPLETED (2025-12-13)**

This plan has been implemented. The key changes:
- HTTP API endpoints (`/changesets`, `/withdrawals`, `/snapshots`) now require `Content-Type: application/cbor`
- Client sends CBOR instead of JSON+base64
- E2E tests updated to use CBOR format
- All 236 unit tests pass

## Summary

Remove all JSON + base64 encoding from the wire format. Use CBOR as the sole encoding format from client through storage:

- **Client → Sequencer**: CBOR (uncompressed)
- **Sequencer → Storage**: CBOR + zstd (batch-level compression)
- **Storage → Validator**: CBOR + zstd

## Data Flow

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                              CLIENT (synddb-client)                         │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                             │
│  SQLite Changeset (raw bytes)                                               │
│         │                                                                   │
│         ▼                                                                   │
│  ┌─────────────────┐                                                        │
│  │ Encode as CBOR  │  ← Simple CBOR struct: {sequence, timestamp, data}     │
│  └────────┬────────┘                                                        │
│           │                                                                 │
│           ▼                                                                 │
│  HTTP POST /changesets                                                      │
│  Content-Type: application/cbor                                             │
│  Body: raw CBOR bytes (NOT compressed - sequencer batch-compresses)         │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                           SEQUENCER (synddb-sequencer)                      │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                             │
│  Receive raw CBOR bytes                                                     │
│         │                                                                   │
│         ▼                                                                   │
│  ┌─────────────────┐                                                        │
│  │ Parse CBOR      │  ← Validate structure, extract fields                  │
│  └────────┬────────┘                                                        │
│           │                                                                 │
│           ▼                                                                 │
│  ┌─────────────────┐                                                        │
│  │ Assign sequence │  ← Monotonic sequence number                           │
│  │ + timestamp     │                                                        │
│  └────────┬────────┘                                                        │
│           │                                                                 │
│           ▼                                                                 │
│  ┌─────────────────┐                                                        │
│  │ Create COSE_Sign1│ ← Sign message with COSE (64-byte sig)                │
│  │ CborSignedMessage│   Protected header: {seq, ts, type}                   │
│  └────────┬────────┘                                                        │
│           │                                                                 │
│           ▼                                                                 │
│  ┌─────────────────┐                                                        │
│  │ Accumulate in   │  ← Batcher collects messages                           │
│  │ CborBatch       │                                                        │
│  └────────┬────────┘                                                        │
│           │                                                                 │
│           ▼  (on flush: size/time threshold)                                │
│  ┌─────────────────┐                                                        │
│  │ Sign batch      │  ← Batch signature over content hash                   │
│  └────────┬────────┘                                                        │
│           │                                                                 │
│           ▼                                                                 │
│  ┌─────────────────┐                                                        │
│  │ Serialize CBOR  │  ← Batch → CBOR bytes                                  │
│  └────────┬────────┘                                                        │
│           │                                                                 │
│           ▼                                                                 │
│  ┌─────────────────┐                                                        │
│  │ Compress (zstd) │  ← Compress entire batch (better ratio)                │
│  └────────┬────────┘                                                        │
│           │                                                                 │
│           ▼                                                                 │
│  Publish to storage (GCS, Local, etc.)                                      │
│  File: {start}_{end}.cbor.zst                                               │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                            STORAGE (GCS / Local)                            │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                             │
│  batches/                                                                   │
│    000000000001_000000000050.cbor.zst  ← CBOR + zstd compressed             │
│    000000000051_000000000100.cbor.zst                                       │
│    ...                                                                      │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                           VALIDATOR (synddb-validator)                      │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                             │
│  Fetch .cbor.zst file                                                       │
│         │                                                                   │
│         ▼                                                                   │
│  ┌─────────────────┐                                                        │
│  │ Decompress zstd │                                                        │
│  └────────┬────────┘                                                        │
│           │                                                                 │
│           ▼                                                                 │
│  ┌─────────────────┐                                                        │
│  │ Parse CBOR      │  ← CborBatch                                           │
│  └────────┬────────┘                                                        │
│           │                                                                 │
│           ▼                                                                 │
│  ┌─────────────────┐                                                        │
│  │ Verify batch sig│  ← 64-byte COSE signature                              │
│  └────────┬────────┘                                                        │
│           │                                                                 │
│           ▼                                                                 │
│  ┌─────────────────┐                                                        │
│  │ Verify each msg │  ← COSE_Sign1 signature per message                    │
│  └────────┬────────┘                                                        │
│           │                                                                 │
│           ▼                                                                 │
│  ┌─────────────────┐                                                        │
│  │ Apply to SQLite │  ← Extract changeset data, apply                       │
│  └─────────────────┘                                                        │
│                                                                             │
│  ┌─────────────────────────────────────────────────────────────────┐        │
│  │ DEBUG UTILITY (optional)                                        │        │
│  │   CborBatch → Pretty JSON for inspection                        │        │
│  └─────────────────────────────────────────────────────────────────┘        │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
```

## Compression Strategy

| Where | What | Compressed? | Rationale |
|-------|------|-------------|-----------|
| Client → Sequencer | Individual message CBOR | **No** | Small overhead, sequencer batch-compresses anyway |
| Sequencer → Storage | Batch CBOR | **Yes (zstd)** | Better compression ratio on batch than individual messages |
| Storage → Validator | Batch CBOR | **Yes (zstd)** | Same file from storage |

Compressing at the batch level gives better compression ratios because zstd can find patterns across multiple messages.

---

## Phase 1: `synddb-shared` (Core Types)

### 1.1 DELETE `serde_helpers.rs`
**File**: `crates/synddb-shared/src/types/serde_helpers.rs`
**Action**: DELETE entire file

No longer needed - was only for JSON + base64 encoding.

### 1.2 Refactor `payloads.rs` - Switch to CBOR
**File**: `crates/synddb-shared/src/types/payloads.rs`

**Changes**:
1. Remove `use super::serde_helpers::base64_serde`
2. Remove all `#[serde(with = "base64_serde")]` attributes
3. Add CBOR serialization methods:
   - `to_cbor(&self) -> Vec<u8>`
   - `from_cbor(bytes: &[u8]) -> Result<Self>`
4. Keep serde derives for debug JSON output if needed, but primary format is CBOR

### 1.3 Refactor `message.rs` - Remove legacy types
**File**: `crates/synddb-shared/src/types/message.rs`

**DELETE**:
- `SignedMessage` struct
- `SignedBatch` struct
- `verify_legacy_signature()`
- `verify_legacy_batch_signature()`
- `compute_signing_payload()` for legacy format
- `compute_messages_hash()` for legacy JSON batch
- `recover_signer()` - 65-byte signature recovery
- `parse_b256()` - only used by legacy verification
- All tests that use legacy format

**KEEP**:
- `MessageType` enum
- `SequenceReceipt` (for HTTP API response - can stay JSON for simplicity)
- `VerificationError` enum
- `parse_cose_protected_header_fields()`
- `build_cose_sig_structure()`
- `verify_secp256k1_without_recovery_id()`

### 1.4 Update `mod.rs`
**File**: `crates/synddb-shared/src/types/mod.rs`

**Changes**:
- Remove `serde_helpers` module export
- Update documentation to reflect CBOR-only format

### 1.5 Keep `cbor/debug.rs`
**File**: `crates/synddb-shared/src/types/cbor/debug.rs`

**Action**: KEEP - used by validator for debugging/pretty-printing

### 1.6 Update `cbor/convert.rs`
**File**: `crates/synddb-shared/src/types/cbor/convert.rs`

**Action**: DELETE or heavily modify
- This converts CBOR → legacy `SignedMessage`/`SignedBatch`
- With legacy types removed, this module is unnecessary

---

## Phase 2: `synddb-sequencer` (Sequencer)

### 2.1 Update `http_api.rs` - Accept CBOR
**File**: `crates/synddb-sequencer/src/http_api.rs`

**Changes**:
1. Change `POST /changesets` to accept `application/cbor`
2. Change `POST /snapshots` to accept `application/cbor`
3. Change `POST /withdrawals` to accept `application/cbor`
4. Parse request body as CBOR instead of JSON
5. Remove `use synddb_shared::types::serde_helpers::base64_serde`
6. Update `MessageResponse` - either remove or switch to CBOR
7. Remove `publisher: Option<Arc<dyn StoragePublisher>>` from `AppState`
8. Keep `SequenceResponse` as JSON (small response, fine for HTTP)

### 2.2 Update `inbox.rs` - Remove SignedMessage
**File**: `crates/synddb-sequencer/src/inbox.rs`

**Changes**:
1. Remove `SignedMessage` creation
2. Return a simpler struct or raw data for the batcher
3. The batcher creates `CborSignedMessage` with COSE signature

### 2.3 Remove legacy publishers
**Files**:
- `crates/synddb-sequencer/src/publish/local.rs`
- `crates/synddb-sequencer/src/publish/gcs.rs`
- `crates/synddb-sequencer/src/publish/mock.rs`

**Changes for `local.rs`**:
- DELETE `LocalPublisher` struct and impl
- DELETE `StoragePublisher` trait impl
- KEEP `LocalTransport` struct and impl
- KEEP `/storage/*` endpoints (serve from CBOR storage)
- Remove `/storage/batches/{id}/json` endpoint

**Changes for `gcs.rs`**:
- DELETE legacy JSON publishing code
- KEEP only CBOR transport code

**Changes for `mock.rs`**:
- DELETE `MockPublisher`
- Create `MockTransport` for testing if needed

### 2.4 Update `traits.rs`
**File**: `crates/synddb-sequencer/src/publish/traits.rs`

**Changes**:
- DELETE `StoragePublisher` trait
- KEEP `TransportPublisher` trait

### 2.5 Update `main.rs`
**File**: `crates/synddb-sequencer/src/main.rs`

**Changes**:
1. Remove legacy publisher initialization
2. Remove `PublisherType` variants for legacy formats
3. Simplify to only use CBOR batcher + transport

---

## Phase 3: `synddb-validator` (Validator)

### 3.1 Simplify `verifier.rs`
**File**: `crates/synddb-validator/src/sync/verifier.rs`

**Changes**:
1. Remove all legacy format comments and code
2. Remove `SignedMessage` usage - work with `CborSignedMessage` directly
3. Simplify to only verify COSE signatures

### 3.2 Update `providers/gcs.rs`
**File**: `crates/synddb-validator/src/sync/providers/gcs.rs`

**Changes**:
1. DELETE `BatchFormat::Json` enum variant
2. DELETE JSON parsing code
3. Only support `.cbor.zst` files
4. Update documentation

### 3.3 Update `providers/http.rs`
**File**: `crates/synddb-validator/src/sync/providers/http.rs`

**Changes**:
1. Fetch CBOR format from `/storage/batches/{start}` (raw CBOR+zstd)
2. Remove JSON parsing
3. Parse response as CBOR+zstd

### 3.4 Update `providers/mock.rs`
**File**: `crates/synddb-validator/src/sync/providers/mock.rs`

**Changes**:
1. Remove `SignedBatch` usage with `cbor_content_hash: None`
2. Create proper `CborBatch` for mock data

### 3.5 Update `validator.rs`
**File**: `crates/synddb-validator/src/validator.rs`

**Changes**:
1. Remove legacy format handling
2. Work with `CborBatch` and `CborSignedMessage` directly
3. Update tests

---

## Phase 4: `synddb-client` (Client)

### 4.1 Update `sender.rs`
**File**: `crates/synddb-client/src/sender.rs`

**Changes**:
1. Encode `ChangesetBatchRequest` as CBOR instead of JSON
2. Send with `Content-Type: application/cbor`

### 4.2 Update `snapshot_sender.rs`
**File**: `crates/synddb-client/src/snapshot_sender.rs`

**Changes**:
1. Encode `SnapshotRequest` as CBOR instead of JSON
2. Send with `Content-Type: application/cbor`

---

## Phase 5: `synddb-benchmark` (Benchmarks)

### 5.1 Update `encoding_benchmark.rs`
**File**: `crates/synddb-benchmark/tests/encoding_benchmark.rs`

**Changes**:
1. DELETE `create_legacy_message()` function
2. DELETE `create_legacy_batch()` function
3. DELETE `sign_legacy()` function
4. DELETE JSON+base64+zstd benchmark section
5. KEEP only CBOR and CBOR+zstd benchmarks

---

## Phase 6: E2E Tests

### 6.1 Update E2E runner client
**File**: `tests/e2e/runner/src/client/sequencer.rs`

**Changes**:
1. Send requests as CBOR instead of JSON
2. Fetch batches as CBOR (remove `/json` endpoint usage)
3. Update response parsing

### 6.2 Update test files
**Files**:
- `tests/e2e/runner/src/tests/storage.rs`
- `tests/e2e/runner/src/tests/snapshot.rs`
- `tests/e2e/runner/src/tests/sqlite_snapshot.rs`

**Changes**:
1. Update to use CBOR format
2. Remove legacy format references

---

## Phase 7: Documentation

**Files**:
- `docs/plans/cbor-cose-implementation.md`
- `SPEC.md`
- `crates/synddb-benchmark/README.md`

**Changes**:
1. Remove all references to JSON + base64 legacy format
2. Document CBOR as the sole format
3. Update HTTP API documentation for CBOR content type

---

## Files Summary

| Action | File |
|--------|------|
| **DELETE** | `crates/synddb-shared/src/types/serde_helpers.rs` |
| **DELETE** | `crates/synddb-shared/src/types/cbor/convert.rs` |
| **MODIFY** | `crates/synddb-shared/src/types/message.rs` |
| **MODIFY** | `crates/synddb-shared/src/types/payloads.rs` |
| **MODIFY** | `crates/synddb-shared/src/types/mod.rs` |
| **MODIFY** | `crates/synddb-sequencer/src/http_api.rs` |
| **MODIFY** | `crates/synddb-sequencer/src/inbox.rs` |
| **MODIFY** | `crates/synddb-sequencer/src/publish/local.rs` |
| **MODIFY** | `crates/synddb-sequencer/src/publish/gcs.rs` |
| **MODIFY** | `crates/synddb-sequencer/src/publish/mock.rs` |
| **MODIFY** | `crates/synddb-sequencer/src/publish/traits.rs` |
| **MODIFY** | `crates/synddb-sequencer/src/main.rs` |
| **MODIFY** | `crates/synddb-validator/src/sync/verifier.rs` |
| **MODIFY** | `crates/synddb-validator/src/sync/providers/gcs.rs` |
| **MODIFY** | `crates/synddb-validator/src/sync/providers/http.rs` |
| **MODIFY** | `crates/synddb-validator/src/sync/providers/mock.rs` |
| **MODIFY** | `crates/synddb-validator/src/validator.rs` |
| **MODIFY** | `crates/synddb-client/src/sender.rs` |
| **MODIFY** | `crates/synddb-client/src/snapshot_sender.rs` |
| **MODIFY** | `crates/synddb-benchmark/tests/encoding_benchmark.rs` |
| **MODIFY** | `tests/e2e/runner/src/client/sequencer.rs` |
| **MODIFY** | `tests/e2e/runner/src/tests/*.rs` |
| **KEEP** | `crates/synddb-shared/src/types/cbor/debug.rs` |

---

## Verification Steps

After each phase:
1. `cargo +nightly fmt --all`
2. `cargo clippy --workspace --all-targets --all-features`
3. `cargo test --workspace`

Final verification:
1. `cargo build --workspace --all-features`
2. Run E2E tests: `docker compose up` + `cargo run -p synddb-e2e`
