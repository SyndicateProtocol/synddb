# Sequencer Node Design Document

## Overview

The Sequencer Node is a standalone binary service responsible for ordering, signing, and persisting SQLite changesets from SyndDB clients. It operates in a separate VM/TEE from the application, providing cryptographic attestation that messages were received and ordered correctly.

## Architecture Context

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                              VM #1 (Application TEE)                        │
│  ┌─────────────────┐    ┌─────────────────┐    ┌─────────────────────────┐  │
│  │   Application   │───▶│  SQLite + SyndDB │───▶│   synddb-client        │  │
│  │   (User Code)   │    │  (Session Ext)   │    │   (Changeset Capture)  │  │
│  └─────────────────┘    └─────────────────┘    └───────────┬─────────────┘  │
│                                                            │ HTTP POST      │
│                              NO SIGNING KEYS               │ /changesets    │
└────────────────────────────────────────────────────────────┼────────────────┘
                                                             │
                                                             ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                              VM #2 (Sequencer TEE)                          │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │                         synddb-sequencer                            │    │
│  │  ┌──────────────┐   ┌──────────────┐   ┌──────────────────────────┐ │    │
│  │  │  HTTP API    │──▶│   Inbox      │──▶│   Signer (secp256k1)     │ │    │
│  │  │  (Axum)      │   │  (Ordering)  │   │   ENV: SIGNING_KEY       │ │    │
│  │  └──────────────┘   └──────────────┘   └───────────┬──────────────┘ │    │
│  │                                                    │                │    │
│  │                                        ┌───────────▼──────────────┐ │    │
│  │                                        │   GCS Publisher          │ │    │
│  │                                        │   (Signed Messages)      │ │    │
│  │                                        └──────────────────────────┘ │    │
│  └─────────────────────────────────────────────────────────────────────┘    │
│                              HAS SIGNING KEY                                │
└─────────────────────────────────────────────────────────────────────────────┘
```

## Core Design Principles

### 1. Single Responsibility: Ordering & Signing

The sequencer's sole job is to:
1. Receive messages (changesets, withdrawal requests)
2. Assign a monotonic sequence number
3. Sign the message with its private key
4. Persist to GCS

It does **NOT**:
- Validate changeset contents
- Execute SQL operations
- Handle Data Availability (DA) beyond GCS
- Filter or reject messages (other than basic format validation)

### 2. Inbox Model (Arbitrum-style)

Similar to Arbitrum's delayed inbox, all messages are accepted and ordered:

```
Message Flow:
  1. Client sends message → HTTP POST
  2. Sequencer assigns sequence_number (monotonic counter)
  3. Sequencer signs: signature = sign(keccak256(sequence || timestamp || message_hash))
  4. Sequencer persists to GCS: gs://bucket/messages/{sequence}.json
  5. Sequencer returns receipt with signature + sequence
```

### 3. Message Types

```rust
/// Inbound messages from clients
pub enum InboundMessage {
    /// SQLite changeset batch from synddb-client
    Changeset {
        batch_id: String,
        changesets: Vec<ChangesetData>,
        attestation_token: Option<String>,  // GCP TEE attestation
    },
    /// Withdrawal request (to be signed and later submitted on-chain)
    Withdrawal {
        request_id: String,
        recipient: Address,      // Ethereum address
        amount: U256,
        data: Vec<u8>,
    },
}

/// Signed message ready for persistence
pub struct SignedMessage {
    pub sequence: u64,
    pub timestamp: u64,           // Unix timestamp (seconds)
    pub message_type: MessageType,
    pub payload: Vec<u8>,         // Serialized InboundMessage
    pub message_hash: B256,       // keccak256(payload)
    pub signature: Signature,     // secp256k1 signature
    pub signer: Address,          // Derived from signing key
}
```

## Crate Structure

The sequencer will be built in the **existing `synddb-sequencer` crate**, which already has stub implementations. The crate structure:

```
crates/synddb-sequencer/
├── Cargo.toml
├── src/
│   ├── lib.rs              # Library exports
│   ├── main.rs             # Binary entry point (NEW)
│   ├── config.rs           # Configuration (exists, needs update)
│   ├── http_api.rs         # Axum HTTP server (exists, needs expansion)
│   ├── inbox.rs            # Message ordering & sequencing (NEW)
│   ├── signer.rs           # secp256k1 signing (NEW)
│   ├── storage/
│   │   ├── mod.rs
│   │   └── gcs.rs          # Google Cloud Storage client (NEW)
│   ├── messages/
│   │   ├── mod.rs          # Message types (exists, needs update)
│   │   ├── changeset.rs    # Changeset message handling
│   │   └── withdrawal.rs   # Withdrawal message handling (NEW)
│   └── monitor/
│       └── mod.rs          # Existing changeset types
```

## Configuration

Following existing conventions (clap + env vars + humantime):

```rust
#[derive(Debug, Clone, Parser)]
#[command(author, version, about = "SyndDB Sequencer Node")]
pub struct SequencerConfig {
    /// HTTP server bind address
    #[arg(long, env = "BIND_ADDRESS", default_value = "0.0.0.0:8433")]
    pub bind_address: SocketAddr,

    /// Private key for signing (hex-encoded, without 0x prefix)
    #[arg(long, env = "SIGNING_KEY")]
    pub signing_key: String,

    /// GCS bucket for message storage
    #[arg(long, env = "GCS_BUCKET")]
    pub gcs_bucket: String,

    /// GCS path prefix for messages
    #[arg(long, env = "GCS_PREFIX", default_value = "messages")]
    pub gcs_prefix: String,

    /// Request timeout
    #[arg(long, env = "REQUEST_TIMEOUT", default_value = "30s", value_parser = parse_duration)]
    pub request_timeout: Duration,

    /// Maximum message size in bytes
    #[arg(long, env = "MAX_MESSAGE_SIZE", default_value = "10485760")]  // 10MB
    pub max_message_size: usize,

    /// Enable TEE attestation verification
    #[arg(long, env = "VERIFY_ATTESTATION", default_value = "false")]
    pub verify_attestation: bool,
}
```

## HTTP API

### Endpoints

| Method | Path | Description |
|--------|------|-------------|
| POST | `/changesets` | Submit changeset batch |
| POST | `/withdrawals` | Submit withdrawal request |
| GET | `/messages/:sequence` | Retrieve signed message by sequence |
| GET | `/health` | Health check |
| GET | `/status` | Sequencer status (sequence number, signer address) |

### Request/Response Formats

**POST /changesets**
```json,norun
// Request
{
  "batch_id": "uuid-v4",
  "changesets": [
    {
      "data": "<base64-encoded>",
      "sequence": 1,
      "timestamp": 1700000000
    }
  ],
  "attestation_token": "optional-jwt"
}

// Response (201 Created)
{
  "sequence": 12345,
  "timestamp": 1700000001,
  "message_hash": "0x...",
  "signature": "0x...",
  "signer": "0x..."
}
```

**POST /withdrawals**
```json,norun
// Request
{
  "request_id": "uuid-v4",
  "recipient": "0x...",
  "amount": "1000000000000000000",
  "data": "<base64-encoded optional>"
}

// Response (201 Created)
{
  "sequence": 12346,
  "timestamp": 1700000002,
  "message_hash": "0x...",
  "signature": "0x...",
  "signer": "0x..."
}
```

## What "Signing Messages" Means

### Cryptographic Signature

The sequencer uses **secp256k1** (Ethereum-compatible) signing:

```rust
use alloy::signers::{LocalWallet, Signer};
use alloy::primitives::{keccak256, B256};

pub struct MessageSigner {
    wallet: LocalWallet,
}

impl MessageSigner {
    pub fn new(private_key: &str) -> Result<Self> {
        let wallet = private_key.parse::<LocalWallet>()?;
        Ok(Self { wallet })
    }

    pub fn address(&self) -> Address {
        self.wallet.address()
    }

    /// Sign a message with EIP-191 prefix for Ethereum compatibility
    pub async fn sign_message(&self, message: &SignedMessage) -> Result<Signature> {
        // Create signing payload
        let payload = self.create_signing_payload(message);

        // Sign with EIP-191 prefix: "\x19Ethereum Signed Message:\n" + len + message
        let signature = self.wallet.sign_message(&payload).await?;

        Ok(signature)
    }

    fn create_signing_payload(&self, msg: &SignedMessage) -> Vec<u8> {
        // Canonical encoding: sequence || timestamp || message_hash
        let mut payload = Vec::new();
        payload.extend_from_slice(&msg.sequence.to_be_bytes());
        payload.extend_from_slice(&msg.timestamp.to_be_bytes());
        payload.extend_from_slice(msg.message_hash.as_slice());
        payload
    }
}
```

### Why Sign?

1. **Proof of Ordering**: The signature proves the sequencer assigned this specific sequence number at this timestamp
2. **Non-repudiation**: The sequencer cannot deny having ordered a message
3. **Verifiability**: Anyone with the signer's address can verify the signature
4. **Bridge Integration**: Signed withdrawals can be submitted to on-chain contracts

### Signature Verification (Client-side or On-chain)

```solidity
// On-chain verification
function verifySequencerSignature(
    uint64 sequence,
    uint64 timestamp,
    bytes32 messageHash,
    bytes memory signature,
    address expectedSigner
) public pure returns (bool) {
    bytes32 payload = keccak256(abi.encodePacked(sequence, timestamp, messageHash));
    bytes32 ethSignedHash = keccak256(abi.encodePacked(
        "\x19Ethereum Signed Message:\n32",
        payload
    ));
    return ECDSA.recover(ethSignedHash, signature) == expectedSigner;
}
```

## GCS Storage

### Storage Layout

```
gs://{bucket}/{prefix}/
├── messages/
│   ├── 000000000001.json
│   ├── 000000000002.json
│   └── ...
├── state/
│   └── sequence.json       # Current sequence number (for recovery)
└── metadata/
    └── signer.json         # Signer address (for verification)
```

### Message Format in GCS

```json
{
  "sequence": 12345,
  "timestamp": 1700000001,
  "message_type": "changeset",
  "payload": "<base64-encoded>",
  "message_hash": "0x...",
  "signature": "0x...",
  "signer": "0x..."
}
```

### GCS Client

Using `google-cloud-storage` crate with workload identity:

```rust
use google_cloud_storage::client::{Client, ClientConfig};

pub struct GcsStorage {
    client: Client,
    bucket: String,
    prefix: String,
}

impl GcsStorage {
    pub async fn new(bucket: String, prefix: String) -> Result<Self> {
        // Uses workload identity or GOOGLE_APPLICATION_CREDENTIALS
        let config = ClientConfig::default().with_auth().await?;
        let client = Client::new(config);
        Ok(Self { client, bucket, prefix })
    }

    pub async fn store_message(&self, msg: &SignedMessage) -> Result<()> {
        let path = format!("{}/messages/{:012}.json", self.prefix, msg.sequence);
        let data = serde_json::to_vec(msg)?;

        self.client
            .upload_object(&self.bucket, &path, data, "application/json")
            .await?;

        Ok(())
    }

    pub async fn get_message(&self, sequence: u64) -> Result<Option<SignedMessage>> {
        let path = format!("{}/messages/{:012}.json", self.prefix, sequence);

        match self.client.download_object(&self.bucket, &path).await {
            Ok(data) => Ok(Some(serde_json::from_slice(&data)?)),
            Err(e) if e.is_not_found() => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}
```

## Integration Points

### 1. synddb-client → Sequencer

**Current**: Client sends to `{sequencer_url}/changesets`
**Change**: None needed - existing HTTP format is compatible

The client's `sender.rs` already sends:
```rust
struct ChangesetBatch {
    changesets: Vec<Changeset>,
    batch_id: String,
    attestation_token: Option<String>,
}
```

### 2. Sequencer → GCS

**New**: Signed messages stored in GCS for:
- Persistence
- Retrieval by validators/replicas
- Audit trail

### 3. Sequencer → Bridge Contract (Future)

**Withdrawals**: Signed withdrawal messages can be submitted to bridge contract:
```solidity
function processWithdrawal(
    uint64 sequence,
    uint64 timestamp,
    address recipient,
    uint256 amount,
    bytes calldata data,
    bytes calldata signature
) external {
    // Verify sequencer signature
    // Process withdrawal
}
```

### 4. Recovery on Startup

The sequencer must recover its state on restart:
1. Read `state/sequence.json` from GCS
2. Verify last N messages are intact
3. Resume from last sequence + 1

## Multi-Phase Implementation Plan

### Phase 1: Core Sequencer ✅ COMPLETE

**Goal**: Minimal viable sequencer that receives, orders, and signs changesets.

**Implemented**:
- `src/main.rs` - Binary entry point with clap CLI
- `src/config.rs` - `SequencerConfig` with env vars support
- `src/inbox.rs` - Monotonic sequence counter with atomic operations
- `src/signer.rs` - secp256k1 signing with alloy
- `src/http_api.rs` - Full REST API (POST /changesets, POST /withdrawals, GET /health, GET /status)
- 28 unit tests covering all components

**Usage**:
```bash
SIGNING_KEY=<hex-private-key> ./synddb-sequencer
```

### Phase 2: GCS Persistence ✅ COMPLETE

**Goal**: Persist all signed messages to GCS with pluggable DA backends.

**Implemented**:
- `src/publish/mod.rs` - `DAPublisher` trait for pluggable backends
- `src/publish/gcs.rs` - Google Cloud Storage implementation
- Publisher integration in HTTP handlers
- State recovery on startup from GCS
- MockPublisher for testing

**Usage**:
```bash
SIGNING_KEY=<key> GCS_BUCKET=my-bucket ./synddb-sequencer --features gcs
```

**Storage Layout**:
```
gs://{bucket}/{prefix}/
├── messages/
│   ├── 000000000001.json
│   ├── 000000000002.json
│   └── ...
└── state/
    └── sequence.json
```

### Phase 3: Withdrawal Support & Message Retrieval ✅ COMPLETE

**Goal**: Full withdrawal support and message retrieval API.

**Implemented**:
- `POST /withdrawals` endpoint with comprehensive validation
  - Ethereum address validation (0x + 40 hex chars)
  - Amount validation (decimal, no leading zeros except "0")
  - Request ID validation (non-empty)
- `GET /messages/:sequence` endpoint for retrieving signed messages
- `MessageResponse` type for API responses
- 8 new tests covering:
  - Valid withdrawal with data
  - Invalid hex in address
  - Invalid amount formats (empty, non-numeric, leading zeros)
  - Empty request_id
  - Zero amount (valid edge case)
  - Message retrieval with publisher
  - Message not found
  - No publisher configured

**Total tests**: 36 passing

### Phase 4: Production Hardening ✅ PARTIAL

**Goal**: Production-ready sequencer.

**Implemented**:
- Structured JSON logging (`LOG_JSON=true` env var)
- Configurable log level (`LOG_LEVEL` env var)
- Graceful shutdown with SIGTERM/SIGINT handling
- State saving before shutdown (with timeout)
- Health check improvements:
  - `GET /health` - Simple liveness probe
  - `GET /ready` - Readiness probe with component checks
- TEE attestation verification:
  - `AttestationVerifier` for validating GCP Confidential Space tokens
  - JWT validation (issuer, audience, expiration)
  - Optional TEE claim validation (secboot, image digest)
  - Integrated into changeset handler (when `VERIFY_ATTESTATION=true`)

**Skipped** (can be added later):
- Prometheus metrics
- Rate limiting (tower-http has this, easy to add)
- Request size limits (tower-http has this, easy to add)

**Pending**:
- Load testing with synddb-benchmark

**Total tests**: 43 passing

### Phase 5: Docker & Deployment (Week 5-6)

**Goal**: Deployable to GCP Confidential VM.

**Tasks**:
1. Create Dockerfile
2. Create docker-compose for local dev
3. GCP deployment configuration
4. Secret management for signing key
5. Documentation for deployment
6. End-to-end integration test

**Deliverables**:
- Docker image
- Deployment documentation
- Running in GCP Confidential Space

## Dependencies

Add to `synddb-sequencer/Cargo.toml`:

```toml
[dependencies]
# Existing
axum = "0.7"
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["json", "env-filter"] }
clap = { version = "4", features = ["derive", "env"] }
humantime-serde = "1"

# New
alloy = { version = "0.3", features = ["signers", "signer-local"] }
google-cloud-storage = "0.17"
uuid = { version = "1", features = ["v4"] }
base64 = "0.21"
thiserror = "1"
tower = { version = "0.4", features = ["limit", "timeout"] }
tower-http = { version = "0.5", features = ["trace", "limit"] }
```

## Security Considerations

1. **Signing Key Protection**: Key passed via env var, never logged, cleared from memory after use
2. **TEE Isolation**: Sequencer runs in separate TEE from application
3. **No Key Extraction**: Signing key never leaves sequencer TEE
4. **Audit Trail**: All messages persisted to GCS with signatures
5. **Rate Limiting**: Prevent DoS via request limits
6. **Size Limits**: Prevent memory exhaustion via max message size

## Open Questions

1. **Sequence Number Format**: Should we use u64 or something more structured (epoch + counter)?
2. **Multi-tenant**: Should one sequencer support multiple applications?
3. **Reorg Handling**: What happens if we need to resequence messages?
4. **Backup Sequencer**: How to handle failover without key sharing?

---

*Document Version: 1.0*
*Last Updated: 2024*
