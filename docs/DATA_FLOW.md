# SyndDB Complete Data Flow Documentation

This document describes all data flows between SyndDB components, including message formats, serialization, and concrete examples.

## System Overview

```
                                    ┌──────────────────────────────────────┐
                                    │         Blockchain (L1)              │
                                    │  ┌─────────────────────────────────┐ │
                                    │  │        Bridge Contract          │ │
                                    │  │ - Deposit events                │ │
                                    │  │ - Withdrawal events             │ │
                                    │  │ - StateSync events              │ │
                                    │  └─────────────┬───────────────────┘ │
                                    └────────────────┼─────────────────────┘
                                                     │
                  ┌──────────────────────────────────┼──────────────────────────────────┐
                  │                                  │                                  │
                  ▼                                  ▼                                  ▼
┌─────────────────────────────────┐   ┌────────────────────────────────┐   ┌─────────────────────────────────┐
│   synddb-chain-monitor          │   │   synddb-client (App TEE)      │   │   Relayer                       │
│   - WebSocket subscription      │   │   - SQLite Session Extension   │   │   - Fetches signatures          │
│   - Event filtering             │   │   - Changeset capture          │   │   - Submits to Bridge           │
│   - Handler dispatch            │   │   - Snapshot generation        │   │                                 │
└──────────────┬──────────────────┘   └──────────────┬─────────────────┘   └──────────────┬──────────────────┘
               │                                     │                                    │
               │ MessageHandler                      │ HTTP POST                          │ HTTP GET
               │ callback                            │ (CBOR)                             │ (JSON)
               ▼                                     ▼                                    │
┌─────────────────────────────────────────────────────────────────────────────────────────┼───────────────────┐
│                                     synddb-sequencer (Sequencer TEE)                    │                   │
│  ┌─────────────────────────┐    ┌─────────────────────────┐    ┌──────────────────────┐ │                   │
│  │    Message Queue        │    │       Inbox             │    │      Batcher         │ │                   │
│  │ - Inbound messages      │◄───│ - Atomic sequencing     │───►│ - Batch grouping     │ │                   │
│  │ - Outbound tracking     │    │ - COSE signing          │    │ - zstd compression   │ │                   │
│  │ - Acknowledgements      │    │ - keccak256 hashing     │    │ - Batch signing      │ │                   │
│  └─────────────────────────┘    └─────────────────────────┘    └──────────┬───────────┘ │                   │
│                                                                            │             │                   │
│  ┌───────────────────────────────────────────────────────────────────────────────────────┼─────────────────┐ │
│  │                                     Publishers                                        │                 │ │
│  │  ┌─────────────────────┐    ┌─────────────────────┐    ┌─────────────────────────┐   │                 │ │
│  │  │   Local (SQLite)    │    │        GCS          │    │    Future: Celestia     │   │                 │ │
│  │  │   - local://N       │    │   - gs://bucket/... │    │    Future: EigenDA      │   │                 │ │
│  │  └─────────────────────┘    └──────────┬──────────┘    └─────────────────────────┘   │                 │ │
│  └───────────────────────────────────────┼──────────────────────────────────────────────┼─────────────────┘ │
└──────────────────────────────────────────┼──────────────────────────────────────────────┼───────────────────┘
                                           │                                              │
                                           │ .cbor.zst files                              │
                                           ▼                                              │
                       ┌─────────────────────────────────────────┐                        │
                       │           Google Cloud Storage          │                        │
                       │                                         │                        │
                       │  gs://{bucket}/{prefix}/batches/        │                        │
                       │   ├── 000000000001_000000000050.cbor.zst│                        │
                       │   ├── 000000000051_000000000100.cbor.zst│                        │
                       │   └── ...                               │                        │
                       └──────────────────┬──────────────────────┘                        │
                                          │                                               │
                                          │ Fetch batches                                 │
                                          ▼                                               │
┌──────────────────────────────────────────────────────────────────────────────────────────────────────────────┐
│                                    synddb-validator                                      │                   │
│  ┌─────────────────────┐   ┌─────────────────────────┐   ┌─────────────────────────────┐ │                   │
│  │     Fetcher         │   │  SignatureVerifier      │   │   ChangesetApplier         │ │                   │
│  │ - HTTP or GCS       │──►│ - COSE verification     │──►│ - SQLite changeset apply   │ │                   │
│  │ - Batch indexing    │   │ - secp256k1/keccak256   │   │ - Schema gap handling      │ │                   │
│  └─────────────────────┘   └─────────────────────────┘   └──────────────┬──────────────┘ │                   │
│                                                                          │               │                   │
│  ┌─────────────────────┐   ┌─────────────────────────┐                  ▼               │                   │
│  │   Bridge Signer     │   │   Signature Store       │        ┌────────────────────┐    │                   │
│  │ - EIP-191 signing   │──►│ - Pending signatures    │◄───────│   StateStore       │    │                   │
│  │ - Withdrawal attest │   │ - Relayer retrieval     │        │ - Sync progress    │    │                   │
│  └─────────────────────┘   └─────────────────────────┘        └────────────────────┘    │                   │
│                                     │                                                    │                   │
│                                     │ GET /signature/{id}                                │                   │
└─────────────────────────────────────┼────────────────────────────────────────────────────┼───────────────────┘
                                      │                                                    │
                                      └────────────────────────────────────────────────────┘
```

---

## 1. Client → Sequencer Data Flow

### 1.1 Changeset Submission

**Origin**: `synddb-client` (Application TEE)
**Destination**: `synddb-sequencer` (Sequencer TEE)
**Protocol**: HTTP POST
**Content-Type**: `application/cbor` (preferred) or `application/json`
**Endpoint**: `POST /changesets`

#### Request: `ChangesetBatchRequest`

```rust
pub struct ChangesetBatchRequest {
    pub batch_id: String,                      // UUID for tracking
    pub changesets: Vec<ChangesetData>,        // Array of changesets
    pub attestation_token: Option<String>,     // TEE attestation (optional)
}

pub struct ChangesetData {
    pub data: Vec<u8>,                         // Raw SQLite changeset bytes
    pub sequence: u64,                         // Client-side sequence
    pub timestamp: u64,                        // Unix seconds
}
```

**Example (CBOR conceptual representation)**:
```json
{
  "batch_id": "550e8400-e29b-41d4-a716-446655440000",
  "changesets": [
    {
      "data": "<binary: SQLite changeset bytes>",
      "sequence": 1,
      "timestamp": 1700000000
    },
    {
      "data": "<binary: SQLite changeset bytes>",
      "sequence": 2,
      "timestamp": 1700000001
    }
  ],
  "attestation_token": "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9..."
}
```

**Example curl command**:
```bash
curl -X POST http://localhost:8433/changesets \
  -H "Content-Type: application/json" \
  -d '{
    "batch_id": "550e8400-e29b-41d4-a716-446655440000",
    "changesets": [
      {
        "data": "SGVsbG8gV29ybGQ=",
        "sequence": 1,
        "timestamp": 1700000000
      }
    ]
  }'
```

#### Response: `SequenceResponse`

```rust
pub struct SequenceResponse {
    pub sequence: u64,                         // Assigned sequence number
    pub timestamp: u64,                        // When sequenced
    pub message_hash: String,                  // keccak256(compressed_payload)
    pub signature: String,                     // 64-byte hex, 0x-prefix
    pub signer: String,                        // 64-byte public key hex
    pub batch_warning: Option<String>,         // If batch publish failed
}
```

**Example response**:
```json
{
  "sequence": 42,
  "timestamp": 1700000050,
  "message_hash": "0x1a2b3c4d5e6f7890abcdef1234567890abcdef1234567890abcdef1234567890",
  "signature": "0x64789012345678901234567890123456789012345678901234567890123456781234567890123456789012345678901234567890123456789012345678901234",
  "signer": "0x83185678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678",
  "batch_warning": null
}
```

---

### 1.2 Withdrawal Submission

**Origin**: `synddb-client` (Application TEE)
**Destination**: `synddb-sequencer` (Sequencer TEE)
**Protocol**: HTTP POST
**Content-Type**: `application/cbor` (preferred) or `application/json`
**Endpoint**: `POST /withdrawals`

#### Request: `WithdrawalRequest`

```rust
pub struct WithdrawalRequest {
    pub request_id: String,                    // Unique request ID
    pub recipient: String,                     // 0x + 40 hex chars
    pub amount: String,                        // Decimal string (big number)
    pub data: Vec<u8>,                         // Optional calldata
}
```

**Example**:
```json
{
  "request_id": "w-123-abc",
  "recipient": "0x742d35Cc6634C0532925a3b844Bc454e4438f44e",
  "amount": "1000000000000000000",
  "data": ""
}
```

#### Response: `WithdrawalResponse`

```rust
pub struct WithdrawalResponse {
    pub sequence: u64,
    pub timestamp: u64,
    pub message_hash: String,                  // COSE message hash
    pub cose_signature: String,                // 64-byte CBOR signature
    pub signer: String,                        // 64-byte public key
    pub bridge_signature: BridgeSignatureResponse,
    pub batch_warning: Option<String>,
}

pub struct BridgeSignatureResponse {
    pub message_id: String,                    // request_id as bytes32
    pub message_hash: String,                  // Bridge message hash
    pub signature: String,                     // 65-byte EIP-191 sig (r||s||v)
    pub signer: String,                        // 20-byte address
    pub sequence: u64,
    pub timestamp: u64,
}
```

**Example response**:
```json
{
  "sequence": 100,
  "timestamp": 1700000100,
  "message_hash": "0xcose1234567890abcdef...",
  "cose_signature": "0x64bytes...",
  "signer": "0x64bytepubkey...",
  "bridge_signature": {
    "message_id": "0x772d3132332d616263000000000000000000000000000000000000000000000000",
    "message_hash": "0xbridgehash...",
    "signature": "0x65bytesig...1b",
    "signer": "0x742d35Cc6634C0532925a3b844Bc454e4438f44e",
    "sequence": 100,
    "timestamp": 1700000100
  },
  "batch_warning": null
}
```

---

### 1.3 Snapshot Submission

**Origin**: `synddb-client` (Application TEE)
**Destination**: `synddb-sequencer` (Sequencer TEE)
**Protocol**: HTTP POST
**Content-Type**: `application/cbor` (preferred) or `application/json`
**Endpoint**: `POST /snapshots`

#### Request: `SnapshotRequest`

```rust
pub struct SnapshotRequest {
    pub snapshot: SnapshotData,
    pub message_id: String,
    pub attestation_token: Option<String>,
}

pub struct SnapshotData {
    pub data: Vec<u8>,                         // Complete SQLite DB bytes
    pub timestamp: u64,
    pub sequence: u64,                         // Changesets included up to
}
```

**Example**:
```json
{
  "snapshot": {
    "data": "U1FMaXRlIGZvcm1hdCAzAAA...",
    "timestamp": 1700000200,
    "sequence": 50
  },
  "message_id": "snap-456",
  "attestation_token": null
}
```

---

## 2. Sequencer Internal Data Flow

### 2.1 Inbox Sequencing

When a message arrives at the sequencer:

1. **Atomic Sequence Assignment**
   ```rust
   let sequence = counter.fetch_add(1, Ordering::SeqCst);
   let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).as_secs();
   ```

2. **Payload Compression**
   ```rust
   let compressed = zstd::encode_all(&payload[..], 3)?;  // level 3
   ```

3. **Message Hash Computation**
   ```rust
   let hash = keccak256(&compressed);  // 32 bytes
   ```

4. **COSE Sign1 Creation**
   - Protected header (CBOR map):
     - Label -65537: sequence number
     - Label -65538: timestamp
     - Standard COSE algorithm identifiers
   - Payload: compressed data
   - Signature: secp256k1 ECDSA over `keccak256(Sig_structure)`

### 2.2 COSE Message Structure

```
COSE_Sign1 = [
    protected : bstr,           // CBOR-encoded header with sequence & timestamp
    unprotected : {             // Map with public key
        -1 : bstr               // 64-byte uncompressed secp256k1 pubkey
    },
    payload : bstr,             // zstd-compressed original payload
    signature : bstr            // 64-byte signature (r || s, no recovery ID)
]
```

**Sig_structure for signing**:
```
Sig_structure = [
    "Signature1",
    protected,                  // CBOR-encoded protected header
    external_aad : "",          // Empty
    payload                     // Compressed payload
]
```

### 2.3 Batch Creation

**Trigger conditions**:
- Message count reaches threshold
- Byte size reaches threshold
- Time interval expires
- Manual flush via `POST /batch/flush`

**Batch structure**:
```rust
pub struct CborBatch {
    pub version: u8,                           // Format version (1)
    pub start_sequence: u64,
    pub end_sequence: u64,
    pub created_at: u64,
    pub content_hash: [u8; 32],                // SHA-256 of concatenated messages
    pub messages: Vec<CborSignedMessage>,
    pub batch_signature: [u8; 64],             // Signs batch payload
    pub pubkey: [u8; 64],
}
```

**Wire format** (CBOR with short keys):
```json
{
  "v": 1,
  "s": 1,
  "e": 50,
  "t": 1700000000,
  "h": "<32-byte SHA-256>",
  "m": ["<cbor msg 1>", "<cbor msg 2>", ...],
  "sig": "<64-byte signature>",
  "pubkey": "<64-byte public key>"
}
```

**Batch signature payload**:
```rust
let payload = keccak256(
    start_sequence.to_be_bytes() ||
    end_sequence.to_be_bytes() ||
    content_hash
);
```

**File naming**: `{start:012}_{end:012}.cbor.zst`
- Example: `000000000001_000000000050.cbor.zst`
- Zero-padded to 12 digits
- Sorted lexicographically in sequence order

---

## 3. Sequencer → Storage Data Flow

### 3.1 GCS Publisher

**Destination**: Google Cloud Storage
**Path**: `gs://{bucket}/{prefix}/batches/{filename}`

**Example paths**:
```
gs://synddb-staging/sequencer/batches/000000000001_000000000050.cbor.zst
gs://synddb-staging/sequencer/batches/000000000051_000000000100.cbor.zst
```

**Upload process**:
1. Serialize batch to CBOR
2. Compress with zstd (level 3)
3. Upload to GCS with content type `application/octet-stream`

### 3.2 Local Publisher

**Destination**: SQLite database
**Path reference**: `local://{sequence}`

Stores messages in SQLite for development/testing:
```sql
CREATE TABLE messages (
    sequence INTEGER PRIMARY KEY,
    message_type TEXT NOT NULL,
    payload BLOB NOT NULL,
    signature BLOB NOT NULL,
    timestamp INTEGER NOT NULL
);
```

---

## 4. Storage → Validator Data Flow

### 4.1 HTTP Fetcher

**Origin**: Sequencer HTTP API
**Destination**: Validator
**Endpoint**: `GET /messages/{sequence}`

**Response**: `MessageResponse`
```rust
pub struct MessageResponse {
    pub sequence: u64,
    pub timestamp: u64,
    pub message_type: String,                  // "changeset"|"withdrawal"|"snapshot"
    pub payload: Vec<u8>,
    pub message_hash: String,
    pub signature: String,
    pub signer: String,
}
```

### 4.2 GCS Fetcher (Batch Mode)

**Origin**: Google Cloud Storage
**Destination**: Validator

**Process**:
1. List objects in `gs://{bucket}/{prefix}/batches/`
2. Parse filenames to build batch index
3. Find batch containing target sequence
4. Download and decompress `.cbor.zst` file
5. Deserialize CBOR batch
6. Extract individual messages

**Batch index entry**:
```rust
pub struct BatchInfo {
    pub start_sequence: u64,
    pub end_sequence: u64,
    pub path: String,                          // GCS object path
    pub content_hash: [u8; 32],
}
```

---

## 5. Validator Internal Data Flow

### 5.1 Signature Verification

```rust
pub struct SignedMessage {
    pub sequence: u64,
    pub timestamp: u64,
    pub message_type: MessageType,
    pub payload: Vec<u8>,                      // Compressed payload
    pub message_hash: String,
    pub signature: String,                     // 64-byte hex
    pub signer: String,                        // 64-byte pubkey hex
    pub cose_protected_header: Vec<u8>,
}
```

**Verification steps**:
1. Parse COSE Sign1 structure
2. Extract protected header → get sequence and timestamp
3. Verify outer fields match header fields
4. Reconstruct `Sig_structure`
5. Compute `keccak256(Sig_structure)`
6. Verify ECDSA signature against expected public key

### 5.2 Changeset Application

After verification:
1. Decompress payload with zstd
2. If `MessageType::Changeset`: apply SQLite changeset
3. If `MessageType::Snapshot`: replace database, reapply pending changesets
4. If `MessageType::Withdrawal`: trigger signing callback
5. Update `StateStore` with new sequence

### 5.3 Bridge Signer (Optional)

When validator processes withdrawals in bridge signer mode:

```rust
pub struct MessageSignature {
    pub message_id: String,                    // 0x-prefixed bytes32
    pub signature: Vec<u8>,                    // 65 bytes (r||s||v)
    pub signer: Address,
    pub signed_at: u64,
}
```

**Signing process**:
1. Convert `request_id` to `message_id` (keccak256 if not hex)
2. Sign with EIP-191 personal sign format
3. Store in `SignatureStore` for relayer retrieval

---

## 6. Validator → Relayer Data Flow

### 6.1 Signature Retrieval API

**Endpoint**: `GET /signature/{message_id}`

**Response** (if found):
```json
{
  "message_id": "0x772d3132332d616263000000000000000000000000000000000000000000000000",
  "signature": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABb",
  "signer": "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
  "signed_at": 1700000000
}
```

### 6.2 List Pending Signatures

**Endpoint**: `GET /pending`

**Response**:
```json
{
  "count": 2,
  "message_ids": [
    "0x1111000000000000000000000000000000000000000000000000000000000000",
    "0x2222000000000000000000000000000000000000000000000000000000000000"
  ]
}
```

### 6.3 Signer Info

**Endpoint**: `GET /info`

**Response**:
```json
{
  "signer": "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
  "bridge_contract": "0x1234567890abcdef1234567890abcdef12345678",
  "chain_id": 84532
}
```

---

## 7. Chain Monitor → Application Data Flow

### 7.1 Blockchain Events

**Origin**: Blockchain (via WebSocket RPC)
**Destination**: Application via `MessageHandler` trait

**Supported events** (examples):
```solidity
event Deposit(
    address indexed from,
    address indexed to,
    uint256 amount,
    bytes data
);

event Withdrawal(
    address indexed from,
    uint256 amount,
    address recipient,
    bytes data
);

event StateSync(
    uint256 indexed blockNumber,
    bytes32 stateRoot,
    bytes proof
);
```

**Handler interface**:
```rust
#[async_trait]
pub trait MessageHandler: Send + Sync {
    async fn handle_event(&self, log: &Log) -> Result<bool>;
    fn event_signature(&self) -> Option<B256>;
    async fn on_start(&self) -> Result<()>;
    async fn on_stop(&self) -> Result<()>;
}
```

**Log structure** (from Alloy):
```rust
pub struct Log {
    pub address: Address,
    pub topics: Vec<B256>,
    pub data: Bytes,
    pub block_hash: Option<B256>,
    pub block_number: Option<u64>,
    pub transaction_hash: Option<B256>,
    pub transaction_index: Option<u64>,
    pub log_index: Option<U256>,
    pub removed: bool,
}
```

---

## 8. Message Queue API (Sequencer)

### 8.1 Inbound Messages

**Push message from chain monitor**:
**Endpoint**: `POST /messages/inbound`

**Request**: `PushInboundRequest`
```json
{
  "message_id": "0x1234...",
  "type": "deposit",
  "payload": {
    "from": "0xabcd...",
    "to": "0xefgh...",
    "amount": "1000000000000000000"
  },
  "sender": "0xabcd...",
  "tx_hash": "0x5678...",
  "block_number": 12345678,
  "confirmations": 12
}
```

**Get pending messages**:
**Endpoint**: `GET /messages/inbound?pending_only=true&limit=100`

**Response**: `GetMessagesResponse`
```json
{
  "messages": [
    {
      "id": 1,
      "message_id": "0x1234...",
      "type": "deposit",
      "payload": {"from": "...", "amount": "..."},
      "sender": "0xabcd...",
      "tx_hash": "0x5678...",
      "block_number": 12345678,
      "confirmations": 12,
      "timestamp": 1700000000,
      "acknowledged": false
    }
  ],
  "has_more": false,
  "latest_id": 1
}
```

### 8.2 Acknowledgement

**Endpoint**: `POST /messages/inbound/{id}/ack`

**Request**: `AckRequest`
```json
{
  "processed": true,
  "note": "Applied to local state"
}
```

**Response**: `AckResponse`
```json
{
  "acknowledged": true,
  "message_id": 1
}
```

---

## 9. Status and Monitoring APIs

### 9.1 Sequencer Status

**Endpoint**: `GET /status`

**Response**: `StatusResponse`
```json
{
  "current_sequence": 42,
  "signer_address": "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
  "signer_pubkey": "0x83185678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678"
}
```

### 9.2 Validator Status

**Endpoint**: `GET /status`

**Response**:
```json
{
  "running": true,
  "last_sequence": 42,
  "last_sync_time": 1700000000,
  "next_sequence": 43
}
```

### 9.3 Batch Statistics

**Endpoint**: `GET /batch/stats`

**Response**: `BatchStatsResponse`
```json
{
  "enabled": true,
  "batches_published": 10,
  "messages_published": 500,
  "bytes_published": 102400,
  "bytes_uncompressed": 512000,
  "compression_ratio": 5.0,
  "pending_messages": 12,
  "pending_bytes": 4096,
  "last_flush_timestamp": 1700000000
}
```

---

## 10. Serialization Reference

### 10.1 Binary Fields

In **JSON**: Binary fields are base64-encoded
```json
{"data": "SGVsbG8gV29ybGQ="}
```

In **CBOR**: Binary fields are raw bytes (major type 2)

### 10.2 Signature Formats

| Format | Length | Description |
|--------|--------|-------------|
| COSE signature | 64 bytes | `r || s` (no recovery ID) |
| EIP-191 signature | 65 bytes | `r || s || v` (Bridge-compatible) |
| Public key | 64 bytes | Uncompressed secp256k1 (no 0x04 prefix) |
| Address | 20 bytes | keccak256(pubkey)[12:32] |

### 10.3 Hash Functions

| Purpose | Algorithm |
|---------|-----------|
| Message hash | keccak256 |
| Batch content hash | SHA-256 |
| Signature digest | keccak256 |

---

## 11. Complete Flow Example

### Deposit → Withdrawal Round Trip

```
1. User deposits ETH to Bridge contract on L1
   ↓
2. Bridge emits Deposit(from, to, amount, data) event
   ↓
3. chain-monitor captures event via WebSocket
   ↓
4. DepositHandler.handle_event() processes deposit
   ↓
5. Handler credits user in application SQLite database
   ↓
6. SQLite Session Extension captures changeset
   ↓
7. synddb-client batches changeset, sends POST /changesets
   ↓
8. synddb-sequencer:
   - Assigns sequence #42
   - Compresses with zstd
   - Signs with COSE/secp256k1
   - Returns SequenceResponse
   ↓
9. Batcher groups messages 1-50 into batch
   ↓
10. Publisher uploads 000000000001_000000000050.cbor.zst to GCS
    ↓
11. synddb-validator fetches batch from GCS
    ↓
12. Validator verifies all signatures
    ↓
13. Validator applies changesets to replica database
    ↓
14. User requests withdrawal via application
    ↓
15. Application calls synddb.submit_withdrawal()
    ↓
16. synddb-client sends POST /withdrawals
    ↓
17. synddb-sequencer:
    - Sequences withdrawal
    - Signs COSE message
    - Also signs Bridge-compatible message
    - Returns WithdrawalResponse with both signatures
    ↓
18. Validator syncs withdrawal message
    ↓
19. Validator bridge_signer signs withdrawal attestation
    ↓
20. Signature stored in SignatureStore
    ↓
21. Relayer polls GET /pending
    ↓
22. Relayer fetches GET /signature/{message_id}
    ↓
23. Relayer submits signature to Bridge.processWithdrawal()
    ↓
24. User receives ETH on L1
```

---

## 12. Environment Variables Reference

### Sequencer
| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `SIGNING_KEY` | Yes | - | secp256k1 private key (hex) |
| `BIND_ADDRESS` | No | `0.0.0.0:8433` | HTTP bind address |
| `PUBLISHER_TYPE` | No | `local` | Storage: `local` or `gcs` |
| `GCS_BUCKET` | No | - | GCS bucket name |
| `GCS_PREFIX` | No | `sequencer` | GCS path prefix |
| `MAX_MESSAGE_SIZE` | No | `10485760` | Max payload (10MB) |
| `VERIFY_ATTESTATION` | No | `false` | Enable TEE attestation |

### Validator
| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `SEQUENCER_PUBKEY` | Yes | - | Expected sequencer public key |
| `FETCHER_TYPE` | No | `http` | Fetcher: `http` or `gcs` |
| `SEQUENCER_URL` | No | - | Sequencer URL (HTTP fetcher) |
| `GCS_BUCKET` | No | - | GCS bucket (GCS fetcher) |
| `BATCH_SYNC_ENABLED` | No | `true` | Use batch sync |
| `BRIDGE_SIGNER` | No | `false` | Enable withdrawal signing |
| `BRIDGE_CONTRACT_ADDRESS` | No | - | Bridge contract address |
| `BRIDGE_CHAIN_ID` | No | - | Target chain ID |

### Client
| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `SEQUENCER_URL` | Yes | - | Sequencer address |
| `BUFFER_SIZE` | No | `100` | Changeset buffer size |
| `MAX_BATCH_SIZE` | No | `1048576` | Max batch bytes (1MB) |
| `PUSH_INTERVAL` | No | `1s` | Auto-push interval |
| `SNAPSHOT_INTERVAL` | No | `100` | Changesets between snapshots |

### Chain Monitor
| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `WS_URLS` | Yes | - | WebSocket RPC URLs |
| `CONTRACT_ADDRESS` | Yes | - | Contract to monitor |
| `START_BLOCK` | No | `0` | Block to start from |
| `EVENT_SIGNATURE` | No | - | Filter specific events |
