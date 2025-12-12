# CBOR/COSE Data Format Implementation Plan

This document outlines a multi-phase engineering plan for implementing CBOR/COSE binary format for batch storage, including a message batching layer and transport layer abstraction.

## Overview

This is a **greenfield refactor** of the data serialization layer. The current JSON+base64 implementation is being replaced with a more efficient CBOR/COSE binary format before production deployment.

## Goals

**Primary:**
1. **Reduce storage costs** by ~40-50% through binary encoding
2. **Enable multi-message batches** (N messages per batch) for efficient storage writes
3. **Provide human-readable inspection** via debug endpoints/tooling (encode/decode symmetry)

**Secondary (lower priority, future work):**
4. **Transport layer extensibility** for future Arweave/ANS-104 support

## Target Architecture

```
Client sends ChangesetBatchRequest (M changesets)
    → CBOR serialize (raw bytes, no base64)
    → zstd compress
    → Wrap in COSE_Sign1 (binary signature envelope)
    → Accumulate in BatchingLayer until threshold
    → Create CborBatch with N COSE_Sign1 messages
    → CBOR serialize batch
    → zstd compress entire batch
    → Pass to Transpo.
    rtPublisher
        ├── GcsTransport: Upload as .cbor.zst
        └── (Future) ArweaveTransport: Wrap in ANS-104, upload via Irys
```

## Key Design Decision: CBOR/COSE as Canonical Format

The CBOR batch is the **canonical storage format**. Transport layers (GCS, Arweave) are wrappers:

- **GCS:** Stores raw CBOR bytes directly
- **Arweave (future):** Wraps CBOR in ANS-104 DataItem with tags for discoverability

This separation means:
- Adding Arweave support doesn't change the core serialization
- Validators can read from any transport layer and get the same `CborBatch`
- Content hashes are consistent across storage backends

## Structure: 1 Batch : N Messages : M Changesets

```
CborBatch (1)
├── version: u8
├── start_sequence: u64
├── end_sequence: u64
├── created_at: u64
├── content_hash: [u8; 32]      // SHA-256 for cross-system addressing
├── batch_signature: [u8; 64]
├── signer: [u8; 20]
└── messages: Vec<CoseSign1Bytes> (N)
    ├── COSE_Sign1[0]
    │   ├── protected: {alg, seq, ts, type}
    │   ├── unprotected: {signer}
    │   ├── payload: zstd bytes (ChangesetBatchRequest with M₀ changesets)
    │   └── signature: [u8; 64]
    └── ...
```

---

## Phase 1: CBOR/COSE Types in synddb-shared

**Goal:** Define binary format types with encode/decode and human-readable conversion.

### 1.1 Add dependencies

**File:** `crates/synddb-shared/Cargo.toml`

```toml
ciborium = "0.2"
coset = "0.3"
serde_bytes = "0.11"
sha2 = "0.10"  # For content hashing
```

### 1.2 Create CBOR message types

**File:** `crates/synddb-shared/src/types/cbor_message.rs` (new)

```rust
/// Message type as integer for compact CBOR encoding
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CborMessageType {
    Changeset = 0,
    Withdrawal = 1,
    Snapshot = 2,
}

/// Parsed contents of a COSE_Sign1 message
#[derive(Debug, Clone)]
pub struct ParsedCoseMessage {
    pub sequence: u64,
    pub timestamp: u64,
    pub message_type: CborMessageType,
    pub payload: Vec<u8>,  // zstd-compressed inner data
    pub signature: [u8; 64],
    pub signer: [u8; 20],
}

/// CBOR-serialized signed message using COSE_Sign1 structure
#[derive(Debug, Clone)]
pub struct CborSignedMessage {
    cose_bytes: Vec<u8>,
}

impl CborSignedMessage {
    /// Create and sign a new message
    pub fn new(
        sequence: u64,
        timestamp: u64,
        message_type: CborMessageType,
        payload: Vec<u8>,
        signer_address: [u8; 20],
        sign_fn: impl FnOnce(&[u8]) -> Result<[u8; 64]>,
    ) -> Result<Self>;

    /// Get raw COSE_Sign1 bytes for storage
    pub fn as_bytes(&self) -> &[u8];

    /// Parse from raw bytes
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self>;

    /// Verify signature and parse contents
    pub fn verify_and_parse(&self, expected_signer: &[u8; 20]) -> Result<ParsedCoseMessage>;

    /// Parse without verification (for debugging)
    pub fn parse_without_verify(&self) -> Result<ParsedCoseMessage>;

    /// Get sequence without full verification (for indexing)
    pub fn sequence(&self) -> Result<u64>;

    /// Convert to human-readable JSON structure (for debugging/inspection)
    pub fn to_json_value(&self) -> Result<serde_json::Value>;

    /// Pretty-print as JSON string
    pub fn to_json_pretty(&self) -> Result<String>;
}

/// CBOR batch containing multiple COSE_Sign1 messages
#[derive(Debug, Clone)]
pub struct CborBatch {
    pub version: u8,
    pub start_sequence: u64,
    pub end_sequence: u64,
    pub created_at: u64,
    pub content_hash: [u8; 32],  // SHA-256 of serialized messages (for Arweave cross-ref)
    pub messages: Vec<CborSignedMessage>,
    pub batch_signature: [u8; 64],
    pub signer: [u8; 20],
}

impl CborBatch {
    /// Create a new batch, computing content hash and signing
    pub fn new(
        messages: Vec<CborSignedMessage>,
        signer_address: [u8; 20],
        sign_fn: impl FnOnce(&[u8]) -> Result<[u8; 64]>,
    ) -> Result<Self>;

    /// Serialize to CBOR bytes (uncompressed)
    pub fn to_cbor(&self) -> Result<Vec<u8>>;

    /// Serialize to CBOR + zstd compressed bytes
    pub fn to_cbor_zstd(&self) -> Result<Vec<u8>>;

    /// Parse from CBOR bytes
    pub fn from_cbor(bytes: &[u8]) -> Result<Self>;

    /// Parse from CBOR + zstd compressed bytes
    pub fn from_cbor_zstd(bytes: &[u8]) -> Result<Self>;

    /// Verify batch signature
    pub fn verify_batch_signature(&self) -> Result<()>;

    /// Verify batch and all message signatures
    pub fn verify_all_signatures(&self) -> Result<()>;

    /// Convert to human-readable JSON structure
    pub fn to_json_value(&self) -> Result<serde_json::Value>;

    /// Pretty-print entire batch as JSON string
    pub fn to_json_pretty(&self) -> Result<String>;

    /// Get content hash as hex string (for Arweave tags)
    pub fn content_hash_hex(&self) -> String;
}
```

### 1.3 COSE_Sign1 helper module

**File:** `crates/synddb-shared/src/types/cose_helpers.rs` (new)

```rust
// Custom header labels for COSE protected header (private use range)
pub const HEADER_SEQUENCE: i64 = -65537;
pub const HEADER_TIMESTAMP: i64 = -65538;
pub const HEADER_MSG_TYPE: i64 = -65539;

/// Build a COSE_Sign1 structure with SyndDB custom headers
pub fn build_cose_sign1(
    sequence: u64,
    timestamp: u64,
    message_type: CborMessageType,
    payload: Vec<u8>,
    signer: [u8; 20],
    sign_fn: impl FnOnce(&[u8]) -> Result<[u8; 64]>,
) -> Result<Vec<u8>>;ok,

/// Parse a COSE_Sign1 structure and extract SyndDB fields
pub fn parse_cose_sign1(bytes: &[u8]) -> Result<ParsedCoseMessage>;

/// Verify COSE_Sign1 signature against expected signer
pub fn verify_cose_sign1(bytes: &[u8], expected_signer: &[u8; 20]) -> Result<()>;

/// Extract sequence from COSE_Sign1 protected header (without full parse)
pub fn extract_sequence(bytes: &[u8]) -> Result<u64>;
```

### 1.4 Human-readable conversion module

**File:** `crates/synddb-shared/src/types/cbor_debug.rs` (new)

```rust
/// Convert CborBatch to a JSON Value for human inspection
pub fn batch_to_json(batch: &CborBatch) -> Result<serde_json::Value> {
    Ok(json!({
        "version": batch.version,
        "start_sequence": batch.start_sequence,
        "end_sequence": batch.end_sequence,
        "created_at": batch.created_at,
        "content_hash": format!("0x{}", hex::encode(batch.content_hash)),
        "batch_signature": format!("0x{}", hex::encode(batch.batch_signature)),
        "signer": format!("0x{}", hex::encode(batch.signer)),
        "message_count": batch.messages.len(),
        "messages": batch.messages.iter()
            .map(|m| message_to_json(m))
            .collect::<Result<Vec<_>>>()?,
    }))
}

/// Convert CborSignedMessage to JSON Value
pub fn message_to_json(msg: &CborSignedMessage) -> Result<serde_json::Value>;

/// Decompress and decode payload to JSON (for deep inspection)
pub fn decode_payload_to_json(payload: &[u8]) -> Result<serde_json::Value>;
```

### Tests

- **Unit:** COSE_Sign1 creation with custom headers
- **Unit:** COSE_Sign1 signature verification (valid and invalid)
- **Unit:** CborBatch serialization roundtrip
- **Unit:** CborBatch with zstd compression roundtrip
- **Unit:** Batch signature verification
- **Unit:** Content hash is deterministic
- **Unit:** `to_json_pretty()` produces valid, readable JSON
- **Unit:** Size comparison vs equivalent JSON (verify ~40% reduction)

---

## Phase 2: Transport Layer Abstraction

**Goal:** Create extensible transport layer for GCS (now) and Arweave (future).

### 2.1 Define TransportPublisher trait

**File:** `crates/synddb-sequencer/src/publish/transport.rs` (new)

```rust
/// Metadata returned after successful publication
#[derive(Debug, Clone)]
pub struct PublishMetadata {
    /// Transport-specific reference (GCS path, Arweave TX ID, etc.)
    pub reference: String,
    /// Content hash of the published data
    pub content_hash: [u8; 32],
}

/// Trait for transport layer implementations
#[async_trait]
pub trait TransportPublisher: Send + Sync + Debug {
    /// Transport name (e.g., "gcs", "arweave")
    fn name(&self) -> &str;

    /// Publish a CBOR batch
    async fn publish(&self, batch: &CborBatch) -> Result<PublishMetadata>;

    /// Retrieve a batch by content hash or sequence range
    async fn fetch(&self, start_sequence: u64) -> Result<Option<CborBatch>>;

    /// List available batches
    async fn list_batches(&self) -> Result<Vec<BatchInfo>>;

    /// Get latest published sequence
    async fn get_latest_sequence(&self) -> Result<Option<u64>>;
}
```

### 2.2 GCS Transport Implementation

**File:** `crates/synddb-sequencer/src/publish/transport_gcs.rs` (new)

```rust
pub struct GcsTransport {
    client: google_cloud_storage::client::Client,
    bucket: String,
    prefix: String,
}

impl TransportPublisher for GcsTransport {
    async fn publish(&self, batch: &CborBatch) -> Result<PublishMetadata> {
        let data = batch.to_cbor_zstd()?;
        let path = format!(
            "{}/batches/{:012}_{:012}.cbor.zst",
            self.prefix, batch.start_sequence, batch.end_sequence
        );

        self.upload(&path, data, "application/cbor+zstd").await?;

        Ok(PublishMetadata {
            reference: format!("gs://{}/{}", self.bucket, path),
            content_hash: batch.content_hash,
        })
    }
}
```

### 2.3 Document future Arweave transport

**File:** `crates/synddb-sequencer/src/publish/transport_arweave.rs` (new, stub)

```rust
//! Future: Arweave transport via ANS-104 bundles
//!
//! When implementing:
//! 1. Add `bundles-rs` dependency
//! 2. Wrap CborBatch in ANS-104 DataItem
//! 3. Add tags: App-Name, Schema-Version, Start-Sequence, End-Sequence, Content-SHA256
//! 4. Upload via Irys/Bundlr for payment delegation
//! 5. Store Arweave TX ID in batch metadata for cross-referencing
//!
//! The CborBatch format is designed to be transport-agnostic:
//! - content_hash enables content-addressed lookup across systems
//! - COSE signatures are independent of transport layer signatures
//! - ANS-104 wrapping adds ~132 bytes overhead per batch (acceptable for batches)

// Placeholder for future implementation
pub struct ArweaveTransport {
    // irys_client: IrysClient,
    // signer: EthereumSigner,
}
```

### Tests

- **Unit:** GcsTransport publishes valid files
- **Unit:** GcsTransport retrieves and parses batches
- **Unit:** Transport trait is object-safe

---

## Phase 3: Batching Layer Infrastructure

**Goal:** Implement message batching in sequencer with comprehensive observability.

### 3.1 Add BatchConfig to sequencer config

**File:** `crates/synddb-sequencer/src/config.rs`

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Parser)]
pub struct BatchConfig {
    /// Maximum messages per batch before flushing (default: 50)
    #[arg(long, env = "BATCH_MAX_MESSAGES", default_value = "50")]
    pub max_messages: usize,

    /// Maximum batch size in bytes before flushing (default: 1MB)
    #[arg(long, env = "BATCH_MAX_BYTES", default_value = "1048576")]
    pub max_batch_bytes: usize,

    /// Maximum time to wait before flushing a partial batch (default: 5s)
    #[arg(long, env = "BATCH_INTERVAL", default_value = "5s", value_parser = humantime::parse_duration)]
    #[serde(with = "humantime_serde")]
    pub batch_interval: Duration,
}
```

**Flush triggers (whichever comes first):**
1. `messages.len() >= max_messages`
2. `accumulated_bytes >= max_batch_bytes`
3. `elapsed >= batch_interval`

### 3.2 Implement Batcher with Structured Logging

**File:** `crates/synddb-sequencer/src/publish/batcher.rs` (new)

```rust
/// Accumulates messages and flushes to transport layer
pub struct Batcher {
    transport: Arc<dyn TransportPublisher>,
    signer: Arc<MessageSigner>,
    config: BatchConfig,
    pending: Mutex<PendingBatch>,
    flush_notify: Notify,
}

struct PendingBatch {
    messages: Vec<CborSignedMessage>,
    total_bytes: usize,
    first_received: Option<Instant>,
}

impl Batcher {
    pub fn new(...) -> Self;

    /// Queue a message for batched publication
    pub async fn queue(&self, message: CborSignedMessage) -> Result<()> {
        let msg_bytes = message.as_bytes().len();
        let sequence = message.sequence()?;

        debug!(
            sequence = sequence,
            message_bytes = msg_bytes,
            "Message queued for batching"
        );

        // ... add to pending ...

        let should_flush = pending.messages.len() >= self.config.max_messages
            || pending.total_bytes >= self.config.max_batch_bytes;

        if should_flush {
            let trigger = if pending.messages.len() >= self.config.max_messages {
                "max_messages"
            } else {
                "max_bytes"
            };
            debug!(
                trigger = trigger,
                message_count = pending.messages.len(),
                total_bytes = pending.total_bytes,
                "Batch threshold reached, triggering flush"
            );
        }
    }

    /// Force flush any pending messages
    pub async fn flush(&self) -> Result<Option<PublishMetadata>> {
        // ... create batch ...

        let batch_bytes = batch.to_cbor_zstd()?.len();
        let wait_duration = pending.first_received.map(|t| t.elapsed());

        info!(
            start_sequence = batch.start_sequence,
            end_sequence = batch.end_sequence,
            message_count = batch.messages.len(),
            uncompressed_bytes = pending.total_bytes,
            compressed_bytes = batch_bytes,
            compression_ratio = format!("{:.1}x", pending.total_bytes as f64 / batch_bytes as f64),
            wait_ms = wait_duration.map(|d| d.as_millis()),
            "Flushing batch to storage"
        );

        let result = self.transport.publish(&batch).await?;

        info!(
            start_sequence = batch.start_sequence,
            end_sequence = batch.end_sequence,
            reference = %result.reference,
            "Batch published successfully"
        );
    }

    /// Background task that flushes on interval
    pub async fn run_flush_loop(&self, shutdown: watch::Receiver<bool>) {
        // On interval flush:
        debug!(
            trigger = "interval",
            message_count = pending.messages.len(),
            total_bytes = pending.total_bytes,
            age_ms = pending.first_received.map(|t| t.elapsed().as_millis()),
            "Interval flush triggered"
        );
    }
}
```

### 3.3 Logging Summary

| Event | Level | Key Fields |
|-------|-------|------------|
| Message queued | `debug` | `sequence`, `message_bytes` |
| Threshold reached | `debug` | `trigger`, `message_count`, `total_bytes` |
| Batch flushing | `info` | `start_sequence`, `end_sequence`, `message_count`, `uncompressed_bytes`, `compressed_bytes`, `compression_ratio`, `wait_ms` |
| Batch published | `info` | `start_sequence`, `end_sequence`, `reference` |
| Interval flush | `debug` | `trigger=interval`, `message_count`, `total_bytes`, `age_ms` |
| Shutdown flush | `info` | `trigger=shutdown`, `message_count` |

This enables operators to:
- Monitor batch sizes and compression efficiency
- Identify which threshold triggers flushes most often
- Track latency from first message to publish
- Tune `max_messages`, `max_batch_bytes`, and `batch_interval` based on observed patterns

### 3.4 Wire into sequencer main

**File:** `crates/synddb-sequencer/src/main.rs`

- Create transport (GCS)
- Create Batcher with transport
- Spawn flush loop task
- Handle graceful shutdown

### 3.4 Update HTTP handlers

**File:** `crates/synddb-sequencer/src/http_api.rs`

- Create `CborSignedMessage` from incoming request
- Queue to `Batcher` instead of direct publish

### 3.5 Add debug endpoint for batch inspection

```rust
/// GET /storage/batches/{start}/debug
/// Returns the batch as pretty-printed JSON for debugging
async fn debug_batch(...) -> Result<impl IntoResponse, HttpError>;
```

### Tests

- **Unit:** Batcher flushes on count threshold
- **Unit:** Batcher flushes on time threshold
- **Unit:** Batcher flushes on shutdown
- **Unit:** Batch signatures are correct with N>1 messages
- **Integration:** Messages queued rapidly produce multi-message batches

---

## Phase 4: Validator CBOR Reader

**Goal:** Validator reads CBOR batches from transport layer.

### 4.1 Update StorageFetcher trait

**File:** `crates/synddb-validator/src/sync/fetcher.rs`

```rust
#[async_trait]
pub trait StorageFetcher: Send + Sync + Debug {
    /// List available batches
    async fn list_batches(&self) -> Result<Vec<BatchInfo>>;

    /// Fetch a batch by start sequence
    async fn fetch_batch(&self, start_sequence: u64) -> Result<Option<CborBatch>>;

    /// Get latest published sequence
    async fn get_latest_sequence(&self) -> Result<Option<u64>>;
}
```

### 4.2 Update GcsFetcher for CBOR with Logging

**File:** `crates/synddb-validator/src/sync/providers/gcs.rs`

```rust
impl StorageFetcher for GcsFetcher {
    async fn fetch_batch(&self, start_sequence: u64) -> Result<Option<CborBatch>> {
        let path = self.find_batch_path(start_sequence).await?;

        debug!(
            start_sequence = start_sequence,
            path = %path,
            "Fetching batch from storage"
        );

        let data = self.download(&path).await?;
        let compressed_bytes = data.len();

        // Decompress and parse
        let batch = CborBatch::from_cbor_zstd(&data)?;

        info!(
            start_sequence = batch.start_sequence,
            end_sequence = batch.end_sequence,
            message_count = batch.messages.len(),
            compressed_bytes = compressed_bytes,
            content_hash = %batch.content_hash_hex(),
            "Batch fetched and parsed"
        );

        // Verify signatures
        batch.verify_all_signatures()?;

        debug!(
            start_sequence = batch.start_sequence,
            "Batch signatures verified"
        );

        Ok(Some(batch))
    }
}

fn parse_batch_filename(filename: &str) -> Option<(u64, u64)> {
    let without_ext = filename.strip_suffix(".cbor.zst")?;
    // ... parse start_end
}
```

### 4.3 Update applier for CborBatch with Logging

**File:** `crates/synddb-validator/src/apply/applier.rs`

```rust
impl Applier {
    pub fn apply_batch(&mut self, batch: &CborBatch) -> Result<()> {
        let start = Instant::now();

        debug!(
            start_sequence = batch.start_sequence,
            end_sequence = batch.end_sequence,
            message_count = batch.messages.len(),
            "Applying batch"
        );

        let mut total_changesets = 0;

        for msg in &batch.messages {
            let parsed = msg.verify_and_parse(&batch.signer)?;
            let changesets_in_msg = self.apply_message(&parsed)?;
            total_changesets += changesets_in_msg;
        }

        info!(
            start_sequence = batch.start_sequence,
            end_sequence = batch.end_sequence,
            message_count = batch.messages.len(),
            total_changesets = total_changesets,
            apply_duration_ms = start.elapsed().as_millis(),
            "Batch applied successfully"
        );

        Ok(())
    }

    fn apply_message(&mut self, msg: &ParsedCoseMessage) -> Result<usize> {
        let decompressed = zstd::decode_all(&msg.payload[..])?;

        debug!(
            sequence = msg.sequence,
            message_type = ?msg.message_type,
            compressed_payload_bytes = msg.payload.len(),
            decompressed_payload_bytes = decompressed.len(),
            "Applying message"
        );

        match msg.message_type {
            CborMessageType::Changeset => {
                let batch: ChangesetBatchRequest = ciborium::from_reader(&decompressed[..])?;
                let changeset_count = batch.changesets.len();

                debug!(
                    sequence = msg.sequence,
                    changeset_count = changeset_count,
                    "Applying changesets from message"
                );

                self.apply_changesets(&batch)?;
                Ok(changeset_count)
            }
            // ...
        }
    }
}
```

### 4.4 Validator Logging Summary

| Event | Level | Key Fields |
|-------|-------|------------|
| Fetching batch | `debug` | `start_sequence`, `path` |
| Batch fetched | `info` | `start_sequence`, `end_sequence`, `message_count`, `compressed_bytes`, `content_hash` |
| Signatures verified | `debug` | `start_sequence` |
| Applying batch | `debug` | `start_sequence`, `end_sequence`, `message_count` |
| Applying message | `debug` | `sequence`, `message_type`, `compressed_payload_bytes`, `decompressed_payload_bytes` |
| Changesets in message | `debug` | `sequence`, `changeset_count` |
| Batch applied | `info` | `start_sequence`, `end_sequence`, `message_count`, `total_changesets`, `apply_duration_ms` |

This enables operators to:
- Track sync progress and throughput
- Monitor decompression ratios
- Identify slow batch applications
- Debug changeset application issues

### 4.5 Add validator debug endpoints

**File:** `crates/synddb-validator/src/http_api.rs`

```rust
/// GET /debug/batch/{sequence}
/// Fetch and decode a batch as pretty-printed JSON
async fn debug_batch(...) -> Result<impl IntoResponse, HttpError>;

/// GET /debug/message/{sequence}
/// Fetch a specific message with decoded payload
async fn debug_message(...) -> Result<impl IntoResponse, HttpError>;
```

### Tests

- **Unit:** GcsFetcher parses .cbor.zst filenames
- **Unit:** GcsFetcher reads and verifies CBOR batches
- **Unit:** Applier processes CborBatch correctly
- **Unit:** Debug endpoints return valid JSON
- **Integration:** Validator syncs CBOR batches from GCS

---

## Phase 5: E2E Testing

**Goal:** Full end-to-end validation with CBOR format and batching.

### 5.1 Update E2E docker-compose

**File:** `tests/e2e-gcs/docker-compose.yml`

```yaml
sequencer:
  environment:
    BATCH_SIZE: "10"
    BATCH_INTERVAL: "2s"
```

### 5.2 Add CBOR/batching E2E tests

**File:** `tests/e2e/runner/src/tests/cbor.rs` (new)

```rust
impl TestRunner {
    /// Test: Batches contain multiple messages when batching enabled
    pub async fn test_multi_message_batch(&self) -> TestCaseResult;

    /// Test: Debug endpoint returns valid JSON
    pub async fn test_debug_endpoint(&self) -> TestCaseResult;

    /// Test: Signature verification works end-to-end
    pub async fn test_signature_verification(&self) -> TestCaseResult;

    /// Test: Payload can be decoded to readable format
    pub async fn test_payload_decode(&self) -> TestCaseResult;

    /// Test: Content hash is consistent
    pub async fn test_content_hash_consistency(&self) -> TestCaseResult;
}
```

### 5.3 Update existing batch tests

**File:** `tests/e2e/runner/src/tests/batch.rs`

- Handle multi-message batches
- Verify content hash field

### Tests

- **E2E:** Full flow: client → sequencer (CBOR) → GCS → validator
- **E2E:** Multi-message batches are created
- **E2E:** Debug endpoints work on both sequencer and validator
- **E2E:** Multi-validator consistency
- **E2E:** Signature verification at all layers

---

## Phase 6: Cleanup & Documentation

### 6.1 Remove old JSON types

- Remove `SignedMessage` and `SignedBatch` from synddb-shared (or deprecate)
- Update all imports

### 6.2 Update SPEC.md

- Document CBOR wire format
- Document COSE_Sign1 structure with custom headers
- Document batch file naming (.cbor.zst)
- Document content hash for cross-system addressing

### 6.3 Document transport layer architecture

- GCS transport details
- Future Arweave integration path
- Content hash usage for cross-referencing

### 6.4 Optional: CLI inspection tool

```bash
# Decode a batch file to JSON
synddb-inspect batch /path/to/batch.cbor.zst

# Decode with full payload contents
synddb-inspect batch /path/to/batch.cbor.zst --decode-payloads
```

---

## Dependencies

```toml
# crates/synddb-shared/Cargo.toml
ciborium = "0.2"
coset = "0.3"
serde_bytes = "0.11"
sha2 = "0.10"

# Future (Phase 7 - Arweave):
# bundles-rs = "0.1"
```

---

## Summary: Encode/Decode Symmetry

| Operation | Sequencer (Encode) | Validator (Decode) |
|-----------|-------------------|-------------------|
| Message → COSE_Sign1 | `CborSignedMessage::new()` | `CborSignedMessage::verify_and_parse()` |
| Batch → CBOR | `CborBatch::to_cbor_zstd()` | `CborBatch::from_cbor_zstd()` |
| CBOR → JSON (debug) | `batch.to_json_pretty()` | `batch.to_json_pretty()` |
| Payload → JSON (debug) | `decode_payload_to_json()` | `decode_payload_to_json()` |
| HTTP debug endpoint | `GET /storage/batches/{id}/debug` | `GET /debug/batch/{id}` |

---

## Future: Arweave Integration (Phase 7)

When adding Arweave support:

1. **Add `bundles-rs` dependency**
2. **Implement `ArweaveTransport`:**
   - Wrap `CborBatch` in ANS-104 DataItem
   - Add tags: `App-Name`, `Schema-Version`, `Start-Sequence`, `End-Sequence`, `Content-SHA256`
   - Upload via Irys/Bundlr
3. **Store cross-references:**
   - Arweave TX ID in GCS object metadata
   - Content hash enables lookup across systems
4. **Add Arweave fetcher to validator:**
   - Query by tags via GraphQL
   - Unwrap ANS-104, parse `CborBatch`

The `content_hash` field in `CborBatch` enables content-addressed lookup across GCS and Arweave, since Arweave uses signature-based addressing (same content = different TX ID).

---

## Implementation Order

| Phase | Depends On | Scope |
|-------|------------|-------|
| 1. CBOR/COSE Types | - | Medium |
| 2. Transport Abstraction | 1 | Small |
| 3. Batching Layer | 1, 2 | Medium |
| 4. Validator CBOR Reader | 1 | Medium |
| 5. E2E Testing | 3, 4 | Medium |
| 6. Cleanup & Docs | 5 | Small |
| 7. Arweave (Future) | 1-6 | Medium |

Phases 1 and 2 can be developed together.
Phase 4 can start after Phase 1 (parallel with 2, 3).
