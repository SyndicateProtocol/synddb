# Validator Implementation Plan

> Implementation details for Message Passing Bridge validators.
> See `SPEC.md` for the specification and `PLAN_VALIDATORS_EXAMPLES.md` for detailed code examples.

## Overview

Validators are off-chain services that:
- Receive messages from applications (Primary)
- Validate messages against schemas and invariants
- Sign valid messages using EIP-712
- Publish messages to storage layers
- Submit signatures to the Bridge contract
- Monitor Bridge events and re-verify messages (Witness)

---

## 1. Validator Architecture

```
validator/
├── main.rs                    # Entry point, config loading
├── config.rs                  # Configuration structs
├── http/
│   ├── server.rs              # Axum HTTP server
│   ├── handlers.rs            # Request handlers
│   └── auth.rs                # mTLS/API key authentication
├── validation/
│   ├── pipeline.rs            # Validation pipeline orchestration
│   ├── schema.rs              # JSON Schema validation
│   ├── calldata.rs            # ABI decoding/validation
│   ├── invariants.rs          # On-chain/oracle invariants
│   └── nonce.rs               # Nonce tracking
├── signing/
│   ├── eip712.rs              # EIP-712 typed data signing
│   └── key_manager.rs         # Key loading/TEE
├── storage/
│   ├── publisher.rs           # Storage layer publication
│   └── fetcher.rs             # Storage layer fetching
├── bridge/
│   ├── client.rs              # Bridge contract interaction
│   └── events.rs              # Event monitoring
└── oracle/
    └── client.rs              # Chainlink/Pyth price feeds
```

---

## 2. Authentication

### Methods

| Method | Use Case |
|--------|----------|
| mTLS | Production TEE-to-TEE |
| API Key | Development/testing |
| TEE Attestation | Header-based verification |

### Config Structure

```rust
pub struct AuthConfig {
    pub method: AuthMethod,
    pub allowed_domains: Vec<[u8; 32]>,
    pub allowed_message_types: Vec<String>,
    pub rate_limit: RateLimitConfig,
}
```

---

## 3. Error Types

```rust
#[derive(Debug, thiserror::Error)]
pub enum ValidationError {
    // Replay & Nonce
    #[error("REPLAY_DETECTED: message {0} already processed")]
    ReplayDetected(String),
    #[error("INVALID_NONCE: expected {expected}, got {provided}")]
    InvalidNonce { domain: String, expected: u64, provided: u64 },

    // Timestamp
    #[error("TIMESTAMP_EXPIRED: message timestamp outside drift window")]
    TimestampExpired { timestamp: u64 },

    // Authorization
    #[error("APP_NOT_AUTHORIZED: domain {0} not registered")]
    AppNotAuthorized(String),
    #[error("MESSAGE_TYPE_NOT_REGISTERED: {0}")]
    MessageTypeNotRegistered(String),

    // Validation
    #[error("CALLDATA_INVALID: {0}")]
    CalldataInvalid(String),
    #[error("SCHEMA_VALIDATION_FAILED: {0}")]
    SchemaValidationFailed(String),

    // Invariants
    #[error("INVARIANT_VIOLATED: {message}")]
    InvariantViolated { invariant: String, message: String },
    #[error("INVARIANT_DATA_STALE: {source} data older than {max_age_seconds}s")]
    InvariantDataStale { source: String, max_age_seconds: u64 },
    #[error("INVARIANT_DATA_UNAVAILABLE: could not fetch {source}")]
    InvariantDataUnavailable { source: String },

    // Storage/Bridge
    #[error("STORAGE_PUBLISH_FAILED: {0}")]
    StoragePublishFailed(String),
    #[error("BRIDGE_SUBMIT_FAILED: {0}")]
    BridgeSubmitFailed(String),
}
```

---

## 4. Validation Pipeline

### Stages

| Stage | Description |
|-------|-------------|
| 1. Replay | Check message_id not processed |
| 2. Nonce | Verify sequential per domain |
| 3. Timestamp | Within MAX_CLOCK_DRIFT (60s) |
| 4. App Auth | Domain registered and active |
| 5. Message Type | Type registered and enabled |
| 6. Calldata | ABI decode matches type |
| 7. Schema | Metadata matches JSON Schema |
| 8. Invariants | On-chain + oracle checks |
| 9. Custom Rules | Rate limits, thresholds |

### Message ID Computation

```rust
pub fn compute_message_id(message: &Message) -> [u8; 32] {
    keccak256(abi.encode(
        message.message_type,
        keccak256(message.calldata),
        message.metadata_hash,
        message.nonce,
        message.timestamp,
        message.domain
    ))
}
```

### Metadata Hash (RFC 8785)

JSON Canonicalization:
1. Object keys sorted by UTF-16 code units
2. No whitespace between tokens
3. Minimal string escaping

---

## 5. Invariant Checking

### Types

| Type | Example |
|------|---------|
| On-Chain | Supply cap, balance checks |
| Oracle | Price deviation, stale data |
| App Logic | Game rules, business constraints |

### Interface

```rust
#[async_trait]
pub trait Invariant: Send + Sync {
    fn name(&self) -> &str;
    async fn check(&self, message: &Message, ctx: &InvariantContext) -> Result<(), ValidationError>;
}
```

---

## 6. Witness Validator

### Flow

1. Subscribe to `MessageInitialized` events
2. Parse `storageRef` from event
3. Fetch message from storage (ar://, ipfs://, gcs://)
4. Verify `messageId` matches computed
5. Re-derive metadata (verify source tx, oracle prices)
6. Check invariants independently
7. Call verification API if configured
8. Sign and submit to Bridge OR reject

### storageRef Parsing

Format: `ar://tx_id|ipfs://QmHash|gcs://bucket/path`

Priority: Arweave > IPFS > GCS

---

## 7. Rejection Flow

### Primary Validator (rejectProposal)

- Consumes nonce (prevents replay)
- Publishes rejection reason to storage
- Emits `ProposalRejected` event

### Witness Validator (rejectMessage)

- Informational only (doesn't block threshold)
- Publishes rejection reason to storage
- Emits `MessageRejected` event

---

## 8. Bridge Client

### Functions

| Function | Caller | Purpose |
|----------|--------|---------|
| `initializeAndSign` | Primary | Initialize + first signature |
| `signMessage` | Witness | Add signature |
| `rejectProposal` | Primary | Reject before init |
| `rejectMessage` | Witness | Reject after init |
| `getLastNonce` | Both | Sync nonce state |
| `subscribe_events` | Witness | Monitor new messages |

---

## 9. HTTP API

### Endpoints

```yaml
POST /messages
  Description: Submit message for validation
  Auth: mTLS or API key
  Request:
    messageType: string
    calldata: hex string
    metadata: object
    nonce: uint64
    timestamp: uint64
    domain: hex string (bytes32)
    value?: string (wei)
  Response (200): { status: "accepted", messageId, signature, storageRef }
  Response (400): { status: "rejected", error: { code, message } }

GET /messages/{messageId}
  Response: { id, status, stage, signaturesCollected, rejectionsCollected, ... }

GET /health
  Response: { healthy, mode, synced, bridgeConnection, ... }

GET /schemas/{messageType}
  Response: { messageType, schema, schemaHash, schemaUri }
```

---

## 10. Storage Publication

### Record Format

```json
{
  "message": {
    "id": "0x...",
    "messageType": "mint(address,uint256)",
    "calldata": "0x...",
    "metadata": { ... },
    "metadataHash": "0x...",
    "nonce": 1,
    "timestamp": 1234567890,
    "domain": "0x..."
  },
  "primarySignature": {
    "validator": "0x...",
    "signature": "0x...",
    "signedAt": 1234567890
  },
  "publication": {
    "publishedBy": "0x...",
    "publishedAt": 1234567890
  }
}
```

### Layers

| Layer | URI Format | Notes |
|-------|------------|-------|
| Arweave | `ar://tx_id` | Permanent, slow |
| IPFS | `ipfs://QmHash` | Decentralized |
| GCS | `gcs://bucket/path` | Fast, centralized |

---

## 11. EIP-712 Signing

### Domain Separator

```solidity
keccak256(abi.encode(
    keccak256("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)"),
    keccak256("SyndBridge"),
    keccak256("1"),
    chainId,
    bridgeAddress
))
```

### Message Typehash

```solidity
keccak256("Message(bytes32 messageId,string messageType,bytes calldata_,bytes32 metadataHash,uint64 nonce,uint64 timestamp,bytes32 domain)")
```

---

## 12. TEE Key Management

### Key Generation (GCP)

```rust
pub async fn generate_key(&self) -> Result<ValidatorKey, Error> {
    let key = LocalSigner::random(&mut OsRng);
    let attestation = self.generate_attestation(&key).await?;
    Ok(ValidatorKey { signer: key, attestation })
}
```

### Attestation Flow

1. Generate key in TEE
2. Request attestation token from platform
3. Register key + attestation on Bridge
4. Bridge verifies attestation

---

## 13. Configuration

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Parser)]
pub struct ValidatorConfig {
    #[arg(long, env = "MODE")]
    pub mode: ValidatorMode,  // Primary or Witness

    #[arg(long, env = "BRIDGE_ADDRESS")]
    pub bridge_address: Address,

    #[arg(long, env = "RPC_URL")]
    pub rpc_url: String,

    #[arg(long, env = "PRIVATE_KEY")]
    pub private_key: Option<String>,

    #[arg(long, env = "MAX_CLOCK_DRIFT", default_value = "60")]
    pub max_clock_drift_seconds: u64,

    // Storage
    pub arweave: Option<ArweaveConfig>,
    pub ipfs: Option<IpfsConfig>,
    pub gcs: Option<GcsConfig>,

    // Oracle
    pub chainlink_rpc: Option<String>,
    pub pyth_endpoint: Option<String>,
}
```

---

## 14. Implementation Checklist

### Primary Validator
- [x] HTTP server with mTLS/API key auth
- [x] Validation pipeline (all 9 stages)
- [x] Schema fetching and caching
- [x] Invariant checking (on-chain, oracle, app logic)
- [x] Nonce tracking with Bridge sync
- [x] EIP-712 signing
- [x] Storage publication (GCS, Memory) - Arweave/IPFS publishers pending
- [x] Bridge interaction (initializeAndSign, rejectProposal)
- [x] Health/status endpoints

### Witness Validator
- [x] Event monitoring (MessageInitialized)
- [x] Storage fetching with fallback (IPFS, Arweave, GCS, HTTP)
- [ ] Metadata re-derivation
- [ ] Verification API client
- [x] Independent invariant checking
- [x] Bridge interaction (signMessage, rejectMessage)

### Shared Components
- [x] Configuration loading
- [ ] TEE key management
- [ ] Oracle client (Chainlink, Pyth)
- [x] RPC client with retry
- [x] Error types and responses
- [x] Logging (structured, JSON)

### Testing
- [x] Unit tests for validation pipeline
- [ ] Integration tests with mock Bridge
- [x] End-to-end tests with testnet
