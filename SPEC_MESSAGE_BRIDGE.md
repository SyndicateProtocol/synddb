# Message Passing Bridge Specification

> **Status**: Draft
> **Version**: 0.1.0
> **Last Updated**: 2025-12-25

## 1. Introduction

### 1.1 Overview

The Message Passing Bridge is a system for validating and executing cross-chain messages with configurable trust models. Applications send typed messages to validators, who validate them against Bridge-defined rules before signing and submitting for on-chain execution.

This specification describes a redesign of the original SQLite replication system to a simpler message validation model that:

1. **Eliminates the client library** - No database hooks or SDK integration required
2. **Eliminates the sequencer** - Validators handle all message processing
3. **Simplifies integration** - Applications just POST messages to an HTTP endpoint
4. **Maintains security** - Bridge smart contract enforces all trust-critical rules

### 1.2 Design Goals

**Simplicity**: Developers integrate by sending HTTP requests with JSON payloads. No libraries to import, no database hooks to configure, no special data formats to learn.

**Flexibility**: Support both single-validator mode (fast, simple, suitable for non-financial use cases) and multi-validator mode (Byzantine fault tolerant, suitable for high-value operations).

**Security through Separation**: The application cannot directly control Bridge funds. The Bridge defines what message types are allowed and what metadata is required. Validators enforce these rules. The application can only request operations within these bounds.

**Progressive Security**: Applications can upgrade their own security posture (running in TEE, better logging, database snapshots) based on the value at risk, without changing the core protocol.

### 1.3 Key Insight

The fundamental security property is: **the application does not control Bridge funds**.

Even if an application is fully compromised, it cannot instruct validators to perform operations outside the Bridge's allowed message types. It cannot bypass schema validation. It cannot forge signatures. The Bridge rules are the trust anchor.

This means more trust can be placed in the application for non-critical decisions, while the Bridge + validators handle the security-critical parts.

---

## 2. Architecture

### 2.1 System Diagram

```
                              APPLICATION
                                  |
                                  | HTTP POST /messages
                                  | { messageType, metadata }
                                  v
+------------------------------------------------------------------+
|                          VALIDATOR(S)                             |
|                                                                   |
|  1. Validate messageType is registered on Bridge                  |
|  2. Validate metadata matches required JSON Schema                |
|  3. Apply custom validation rules (rate limits, invariants)       |
|  4. Sign message with TEE-protected key                           |
|  5. Publish to DA layer (audit trail)                             |
|  6. Submit signature to Bridge                                    |
|                                                                   |
+------------------------------------------------------------------+
        |                                           |
        | Signatures                                | Audit Trail
        v                                           v
+------------------+                      +------------------+
|     BRIDGE       |                      |    DA LAYER      |
|                  |                      |                  |
| - Message type   |                      | - All validated  |
|   registry       |                      |   messages       |
| - Schema hashes  |                      | - Historical     |
| - Sig threshold  |                      |   replay         |
| - Pre/post       |                      | - Dispute        |
|   modules        |                      |   resolution     |
|                  |                      |                  |
+------------------+                      +------------------+
        |
        | Execute
        v
+------------------+
| TARGET CONTRACT  |
| (mint, transfer, |
|  withdraw, etc.) |
+------------------+
```

### 2.2 Comparison with Previous Architecture

| Aspect             | SQLite Replication (Old)   | Message Passing (New) |
| ------------------ | -------------------------- | --------------------- |
| Client Integration | Required SDK, SQLite hooks | HTTP POST             |
| Data Format        | SQLite changesets (binary) | JSON + ABI signature  |
| Sequencer          | Required (separate TEE)    | Eliminated            |
| Message Origin     | Database writes            | Direct API calls      |
| Validation         | SQL replay + invariants    | Schema + custom rules |
| Developer Effort   | High (library integration) | Low (HTTP calls)      |

### 2.3 Component Responsibilities

**Application**
- Constructs messages with calldata and metadata
- Authenticates to Primary Validator (see Section 2.4)
- Sends messages via HTTP POST
- Responsible for its own security based on value at risk

**Validator (Two Modes)**

Validators operate in one of two modes:

| Mode | Role | Data Flow |
|------|------|-----------|
| **Primary Validator** | Connected directly to application | Receives messages via HTTP, publishes to storage layer |
| **Witness Validator** | Independent verification | Reads messages from storage layer, verifies independently |

*Primary Validator*:
- Receives messages directly from authenticated application
- Validates calldata and metadata against schemas
- Signs valid messages
- **Publishes to storage layer** (Arweave, IPFS, GCS) for other validators
- Submits signature to Bridge

*Witness Validator*:
- **Reads messages from storage layer** (does NOT receive from application)
- Independently validates calldata and metadata
- Re-derives verifiable metadata when possible (e.g., checks prices via public APIs)
- Signs valid messages
- Submits signature to Bridge

This separation ensures witness validators can independently verify without trusting the primary validator's HTTP connection.

**Bridge (Smart Contract)**
- Maintains registry of allowed message types
- Stores metadata schema hashes (schemas on IPFS/Arweave)
- Aggregates validator signatures (on-chain)
- Enforces signature threshold
- Executes messages via modular pre/post hooks
- Controls funds and enforces security invariants

**Storage Layer vs DA Layer**

We distinguish between two types of external data layers:

| Type | Purpose | Examples | Recommendation |
|------|---------|----------|----------------|
| **Storage Layer** | Long-term archival, schema storage, audit trail | Arweave, IPFS, GCS | **Required** - at least one |
| **DA Layer** | Short-term data availability guarantees | Celestia, EigenDA | Optional - for high-throughput |

**Recommended configuration**:
- **Storage** (required): Arweave or IPFS with pinning for permanent schema storage and message archival
- **DA** (optional): Celestia for applications requiring short-term availability guarantees

### 2.4 Application Authentication

Securing the application-to-validator connection is critical. The Primary Validator must verify that incoming messages originate from an authorized application.

**Authentication Methods**:

| Method | Security Level | Use Case |
|--------|---------------|----------|
| API Key + HTTPS | Basic | Development, internal tools |
| mTLS (Mutual TLS) | High | Production applications |
| TEE Attestation | Highest | Financial applications |

**Recommended: mTLS with Application Registration**

```
1. Application generates TLS client certificate
2. Application registers with validator:
   POST /register
   {
     "appId": "0x...",
     "publicKey": "-----BEGIN CERTIFICATE-----...",
     "allowedMessageTypes": ["mint(address,uint256)", ...],
     "rateLimit": { "maxPerSecond": 100, "maxPerDay": 10000 }
   }
3. Validator stores registration, assigns appId
4. All subsequent requests use mTLS with client cert
5. Validator verifies: cert → appId → authorized message types
```

**For TEE-based Applications**:

Applications running in TEE can include attestation tokens:

```
POST /messages
Headers:
  X-TEE-Attestation: <attestation_token>
  X-TEE-Platform: gcp-confidential-space

Body: { message... }
```

The validator verifies:
1. Attestation token is valid and recent
2. Application code hash matches registered hash
3. TEE platform is trusted

This provides cryptographic proof of application integrity, not just identity.

### 2.5 Application Registration

Applications must be registered before they can submit messages.

**Registration Flow**:

```
1. Application operator deploys/configures their application
2. Application operator contacts Primary Validator operator (or self-operates)
3. Primary Validator registers application on Bridge:
   Bridge.registerApplication(appId, primaryValidator, config)
4. Application receives appId and can begin submitting messages
```

**Recommended: Deterministic appId**

We recommend deriving `appId` deterministically to avoid coordination:

```solidity
// Option A: From application's on-chain address
bytes32 appId = keccak256(abi.encode(chainId, applicationAddress));

// Option B: From domain name (for off-chain apps)
bytes32 appId = keccak256(abi.encode("app.example.com"));

// Option C: From Primary Validator + nonce (for multi-tenant validators)
bytes32 appId = keccak256(abi.encode(primaryValidator, registrationNonce));
```

**Bridge Registration Function**:

```solidity
struct ApplicationConfig {
    address primaryValidator;      // Who can initialize messages
    uint64 expirationSeconds;      // Message expiration (default: 86400 = 24h)
    bool requireWitnessSignatures; // Require multi-validator mode
}

function registerApplication(
    bytes32 appId,
    ApplicationConfig calldata config
) external;

function getApplicationConfig(bytes32 appId)
    external view returns (ApplicationConfig memory);
```

### 2.6 Nonce Management

Nonces prevent replay and ensure ordering. The Primary Validator is the sole authority on nonce tracking.

**Semantics** (follows Ethereum transaction nonces):

1. Nonces must be strictly sequential (no gaps)
2. A nonce is **consumed** when the message reaches any terminal state:
   - `Completed` (execution succeeded)
   - `Failed` (execution reverted)
   - `Expired` (threshold not reached in time)
3. Validator rejections do NOT consume nonces (message never initialized)
4. Once initialized on Bridge, the nonce is consumed regardless of outcome

**No Gap Tolerance**:

```
Valid:   nonce 1 → nonce 2 → nonce 3
Invalid: nonce 1 → nonce 3 (gap at 2)
```

If a message fails or expires, the application must continue with the next nonce. There is no retry-with-same-nonce mechanism.

**Primary Validator Nonce Tracking**:

```python
class NonceTracker:
    def __init__(self):
        self.last_nonce = {}  # appId -> last accepted nonce

    def validate_nonce(self, app_id: str, nonce: int) -> bool:
        expected = self.last_nonce.get(app_id, 0) + 1
        return nonce == expected

    def consume_nonce(self, app_id: str, nonce: int):
        """Called when message is initialized on Bridge"""
        self.last_nonce[app_id] = nonce
```

### 2.7 Message Expiration

Messages expire if they don't reach signature threshold within the expiration window.

**Default**: 24 hours from message timestamp

**Expiration Check**:

```solidity
function isExpired(bytes32 messageId) public view returns (bool) {
    MessageState storage state = messageStates[messageId];
    ApplicationConfig storage config = applicationConfigs[state.appId];

    uint256 expirationTime = state.timestamp + config.expirationSeconds;
    return block.timestamp > expirationTime;
}

function expireMessage(bytes32 messageId) external {
    require(isExpired(messageId), "Not expired");
    require(messageStates[messageId].stage == MessageStage.Pending, "Not pending");

    messageStates[messageId].stage = MessageStage.Expired;
    emit MessageExpired(messageId, block.timestamp);
}
```

**Expiration as Terminal State**:

- Expired messages cannot be signed or executed
- Nonce is consumed (no retry with same nonce)
- Application must submit new message with new nonce
- Expiration reason logged on-chain for audit

---

## 3. Message Format

### 3.1 Key Distinction: Calldata vs Metadata

This specification distinguishes between two types of data in a message:

**Calldata**: The actual function parameters that will be executed on-chain. Uses standard Ethereum ABI encoding. For `mint(address,uint256)`, this is the recipient address and amount. Calldata is self-describing via the function signature.

**Metadata**: Additional "evidence" the application provides to convince validators that the requested operation is legitimate. This includes context like `reason`, `sourceChain`, `sourceTxHash`, invariant proofs, and any other information validators need to verify the request. Metadata is NOT executed on-chain - it's purely for validator decision-making.

Think of validators as independent entities that the application must convince. The metadata is the application's argument for why validators should sign.

### 3.2 Message Structure

```typescript
interface Message {
  // Unique identifier (hash of content, prevents replay)
  id: bytes32;

  // ABI function signature (e.g., "mint(address,uint256)")
  messageType: string;

  // Standard Ethereum ABI-encoded function parameters
  // This is what gets executed on the target contract
  calldata: bytes;

  // Evidence for validators - NOT executed on-chain
  // Schema retrieved from Bridge via getMessageTypeConfig(messageType).schemaUri
  metadata: object;

  // Monotonically increasing per-application nonce
  nonce: uint64;

  // Unix timestamp in seconds
  timestamp: uint64;

  // Application identifier - each application registers with validators
  // and receives a unique appId for message attribution and rate limiting
  appId: bytes32;

  // Native token amount to send with execution (optional)
  value?: uint256;
}
```

**Finding the Metadata Schema**: Applications query the Bridge contract's `getMessageTypeConfig(messageType)` function, which returns the `schemaUri` (IPFS/Arweave link) or `schemaHash` for on-chain lookup. Validators cache these schemas and validate incoming metadata against them.

### 3.3 Message Type Format

Message types use standard Solidity function signature format:

```
messageType = "functionName(type1,type2,...)"
```

Examples:
- `mint(address,uint256)` - Mint tokens to address
- `transfer(address,address,uint256)` - Transfer between addresses
- `safeMint(address,uint256,string)` - Mint NFT with URI
- `batchTransfer(address[],uint256[])` - Batch transfer

The 4-byte function selector is: `bytes4(keccak256(messageType))`

Function signatures are stable and standardized (e.g., ERC-20 `transfer` is always `transfer(address,uint256)`). What evolves over time is the metadata schema - as applications add more invariants or context for validators.

### 3.4 Calldata (Function Parameters)

Calldata contains the actual function arguments, ABI-encoded:

```solidity
// For mint(address,uint256)
bytes memory calldata = abi.encodeWithSelector(
    bytes4(keccak256("mint(address,uint256)")),
    recipientAddress,  // address parameter
    amount             // uint256 parameter
);
```

This is standard Ethereum encoding - validators and the Bridge don't need special logic to interpret it.

### 3.5 Metadata (Validator Evidence)

Metadata is the application's evidence to convince validators. It's validated against a JSON Schema but NOT executed on-chain.

**Example: ERC20 Mint Message**

```json
{
  "id": "0x1234567890abcdef...",
  "messageType": "mint(address,uint256)",
  "calldata": "0x40c10f19000000000000000000000000742d35cc6634c0532925a3b844bc454e4438f44e0000000000000000000000000000000000000000000000000de0b6b3a7640000",
  "metadata": {
    "reason": "user_deposit",
    "sourceChain": "ethereum",
    "sourceTxHash": "0xabcdef1234567890...",
    "depositAmount": "1000000000000000000",
    "userTotalMinted": "5000000000000000000",
    "contractTotalSupply": "1000000000000000000000000",
    "maxSupply": "10000000000000000000000000"
  },
  "nonce": 42,
  "timestamp": 1735084800,
  "appId": "0x0000000000000000000000000000000000000001"
}
```

Note: The calldata contains the ABI-encoded `mint(0x742d..., 1000000000000000000)`. The metadata provides context validators use to decide whether to sign.

### 3.6 Invariant Specification

Invariants are constraints that must hold true. They can be specified at two levels:

**Schema-Level Invariants** (in JSON Schema):
- Required fields and their types
- Value ranges and patterns
- Field relationships

**Application-Reported Invariants** (in metadata):
- Current state values (e.g., `userTotalMinted`, `contractTotalSupply`)
- Limits (e.g., `maxSupply`, `maxPerUser`)
- Pre-computed checks the validator can verify

**Example: NFT with Per-User and Global Limits**

```json
{
  "messageType": "safeMint(address,uint256,string)",
  "calldata": "0x...",
  "metadata": {
    "recipient": "0x742d35Cc6634C0532925a3b844Bc454e4438f44e",
    "tokenId": "12345",

    "userCurrentBalance": 2,
    "maxPerUser": 3,

    "currentTotalSupply": 9500,
    "maxTotalSupply": 10000,

    "reason": "game_reward",
    "gameSessionId": "session_abc123",
    "achievementId": "first_win"
  }
}
```

Validators verify:
1. `userCurrentBalance + 1 <= maxPerUser` (user can receive this NFT)
2. `currentTotalSupply + 1 <= maxTotalSupply` (supply cap not exceeded)
3. Additional custom rules (e.g., verify game session via external API)

The schema defines which invariant fields are required; validators enforce the logic.

### 3.7 Message ID Generation

Message IDs follow standard Ethereum hashing patterns:

```solidity
bytes32 messageId = keccak256(abi.encode(
    messageType,
    keccak256(calldata),
    keccak256(abi.encode(metadata)),  // Metadata included in ID
    nonce,
    timestamp,
    appId
));
```

This is consistent with how Ethereum transactions derive IDs - hashing the full content ensures uniqueness and tamper detection.

### 3.8 JSON Canonicalization

For deterministic message ID generation, metadata must be serialized consistently. We use **RFC 8785 (JCS - JSON Canonicalization Scheme)**.

**Canonicalization Rules**:

1. **Key ordering**: Object keys sorted lexicographically by UTF-16 code units
2. **No whitespace**: No spaces or newlines between tokens
3. **Number formatting**: No leading zeros, no trailing zeros after decimal, no positive sign
4. **String escaping**: Minimal escaping (only required characters)
5. **No duplicate keys**: Each key appears once per object

**Example**:

```json
// Input (non-canonical)
{
  "amount": "1000",
  "reason": "deposit",
  "recipient": "0x123..."
}

// Canonical output
{"amount":"1000","reason":"deposit","recipient":"0x123..."}
```

**Computing metadataHash**:

```python
import json
import hashlib

def canonicalize(obj):
    """RFC 8785 JSON Canonicalization"""
    return json.dumps(obj, separators=(',', ':'), sort_keys=True, ensure_ascii=False)

def compute_metadata_hash(metadata: dict) -> bytes:
    canonical = canonicalize(metadata)
    return hashlib.sha3_256(canonical.encode('utf-8')).digest()  # keccak256
```

**Edge Cases to Watch**:

| Case | Handling |
|------|----------|
| Unicode strings | UTF-8 encoding, no BOM |
| Large integers | Use string representation (e.g., `"1000000000000000000"`) |
| Floating point | Avoid floats; use string decimals if needed |
| Null values | Include as `null`, don't omit |
| Empty objects/arrays | Include as `{}` or `[]` |
| Nested objects | Recursively canonicalize |

**Validation**:

Validators MUST verify that `metadataHash` matches the canonical hash of the metadata before signing. Witness Validators fetch metadata from `storageRef` and independently compute the hash.

---

## 4. Validation Protocol

### 4.1 Two Validator Flows

The validation flow differs based on validator mode:

**Primary Validator Flow** (receives from application):
```
HTTP Receive → Authenticate → Validate → Sign → Publish to Storage → Submit to Bridge
```

**Witness Validator Flow** (reads from storage):
```
Read from Storage → Validate → Re-derive Metadata → Sign → Submit to Bridge
```

### 4.2 Primary Validator Processing

The Primary Validator is the single validator connected to the application via HTTP.

**Stage 1: Receive and Authenticate**

```
POST /messages
Authorization: <mTLS client cert or API key>
Content-Type: application/json

{
  "messageType": "mint(address,uint256)",
  "calldata": "0x40c10f19...",
  "metadata": { ... },
  "nonce": 42,
  "timestamp": 1735084800,
  "appId": "0x..."
}
```

The Primary Validator verifies application identity (see Section 2.4).

**Stage 2: Validate**

```
┌─────────────────────────────────────────────────────────────────┐
│                     VALIDATION PIPELINE                          │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  2.1 REPLAY PROTECTION                                           │
│      ├─ Compute message ID from content                          │
│      ├─ Check: ID not already processed                          │
│      └─ Check: nonce > lastNonce[appId]                          │
│                                                                  │
│  2.2 FRESHNESS CHECK                                             │
│      └─ Check: |timestamp - now| < MAX_CLOCK_DRIFT               │
│                                                                  │
│  2.3 APPLICATION AUTHORIZATION                                   │
│      └─ Check: appId is registered and active                    │
│                                                                  │
│  2.4 MESSAGE TYPE VALIDATION                                     │
│      ├─ Query Bridge: getMessageTypeConfig(messageType)          │
│      ├─ Check: message type is registered                        │
│      └─ Check: message type is enabled                           │
│                                                                  │
│  2.5 CALLDATA VALIDATION                                         │
│      ├─ Decode calldata using messageType ABI                    │
│      └─ Check: calldata matches expected parameter types         │
│                                                                  │
│  2.6 METADATA SCHEMA VALIDATION                                  │
│      ├─ Fetch schema (from cache or IPFS/Arweave)                │
│      ├─ Validate metadata against JSON Schema                    │
│      └─ Check: all required evidence fields present              │
│                                                                  │
│  2.7 INVARIANT CHECKS                                            │
│      ├─ Verify metadata invariants (e.g., supply limits)         │
│      └─ Check: invariant conditions hold                         │
│                                                                  │
│  2.8 CUSTOM RULES (validator-specific)                           │
│      ├─ Rate limiting (messages per second/minute)               │
│      ├─ Amount thresholds (flag large transfers)                 │
│      └─ External verification (check source chain, etc.)         │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

**Primary Rejection Path**:

If any validation check fails, the Primary Validator rejects the message. This is an **implicit rejection** - the message is never initialized on-chain.

```
Validation Failed
      │
      ▼
┌─────────────────────────────────────────────────────────────────┐
│                     PRIMARY REJECTION                            │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  1. Return error response to application (HTTP 400/422)          │
│                                                                  │
│  2. Log rejection locally (for audit)                            │
│     - Message content                                            │
│     - Rejection reason                                           │
│     - Timestamp                                                  │
│                                                                  │
│  3. Optionally publish rejection to storage layer                │
│     - For audit trail                                            │
│     - Not required for protocol                                  │
│                                                                  │
│  4. Do NOT:                                                      │
│     - Call initializeMessage() on Bridge                         │
│     - Call rejectMessage() on Bridge (message doesn't exist)     │
│     - Consume the nonce                                          │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

**Key distinction**: Primary rejection is implicit (no on-chain state), while Witness rejection is explicit (calls `rejectMessage()` on-chain to log dissent for an already-initialized message).

**Nonce behavior on Primary rejection**:
- Nonce is NOT consumed
- Application can retry with the same nonce after fixing the issue
- This differs from on-chain failures where nonce is consumed

**Stage 3: Sign**

We use **EIP-712 typed data signing** for structured, verifiable signatures:

```solidity
bytes32 DOMAIN_SEPARATOR = keccak256(abi.encode(
    keccak256("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)"),
    keccak256("SyndBridge"),
    keccak256("1"),
    chainId,
    bridgeAddress
));

bytes32 structHash = keccak256(abi.encode(
    MESSAGE_TYPEHASH,
    messageId,
    keccak256(bytes(messageType)),
    keccak256(calldata),
    keccak256(abi.encode(metadata)),
    nonce,
    timestamp,
    appId
));

bytes32 digest = keccak256(abi.encodePacked("\x19\x01", DOMAIN_SEPARATOR, structHash));
signature = sign(digest, validatorPrivateKey);
```

Validators are registered on the Bridge during TEE bootstrapping (see Section 6.9). No per-message TEE attestation is required - the validator's signing key is already attested.

**Stage 4: Publish to Storage Layer**

**Only the Primary Validator publishes to storage.** This is critical for the Witness flow.

```
1. Serialize: message + signature + storageReference
2. Publish to configured storage (Arweave, IPFS, GCS)
3. Record storage reference locally
```

Witness Validators will read from this storage layer.

**Stage 5: Submit to Bridge**

Submit signature for on-chain aggregation:

```solidity
Bridge.signMessage(messageId, signature)
```

### 4.3 Witness Validator Processing

Witness Validators provide independent verification without trusting the Primary Validator's HTTP connection.

**Stage 1: Read from Storage**

Poll or subscribe to the storage layer for new messages:

```
1. Watch storage layer for new entries
2. Fetch message: calldata, metadata, primary signature
3. Verify primary signature is from registered Primary Validator
```

**Stage 2: Validate (Same as Primary)**

Run the same validation pipeline as the Primary Validator.

**Stage 3: Re-derive Verifiable Metadata**

Witness Validators should independently verify metadata claims when possible:

```
┌─────────────────────────────────────────────────────────────────┐
│                    METADATA RE-DERIVATION                        │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  For each metadata field:                                        │
│                                                                  │
│  - sourceTxHash: Query source chain RPC to verify tx exists     │
│  - depositAmount: Verify amount in source chain event logs      │
│  - price: Query public price API (Chainlink, CoinGecko)         │
│  - currentSupply: Query target contract view function            │
│  - userBalance: Query target contract balanceOf()                │
│                                                                  │
│  If re-derived value differs from metadata:                      │
│    → REJECT message                                              │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

This is the key security benefit of Witness Validators - they don't just trust the application's claims.

**Stage 4: Sign and Submit**

Same as Primary Validator (Stage 3 + Stage 5), but **skip storage publication**.

### 4.4 Witness Discovery via storageRef

Witness Validators discover messages through the `storageRef` field, which is included in the `MessageInitialized` event on the Bridge.

**storageRef URI Format**:

The `storageRef` is a URI that tells Witness Validators where to fetch the full message and metadata. Multiple storage locations can be specified for redundancy:

```
# Single storage layer
ar://tx_id_here
ipfs://QmHash...
gcs://bucket/path/to/message.json

# Multiple storage layers (separated by |)
ar://tx_id_here|ipfs://QmHash...|gcs://bucket/path

# With multiple Primary Validators (each publishes to their own storage)
ar://primary1_tx|ar://primary2_tx
```

**Witness Discovery Flow**:

```
1. Witness watches for MessageInitialized events on Bridge
2. Extract storageRef from event: "ar://abc123|ipfs://Qm..."
3. Parse URI(s) and attempt to fetch from each in order
4. Verify fetched content:
   - messageId matches event
   - metadataHash matches hash of fetched metadata
   - Primary signature is valid
5. If fetch fails from all URIs → wait and retry (storage propagation delay)
6. If content hash mismatch → reject message
7. Proceed with validation
```

**Storage Layer Priorities**:

Witness Validators should try storage layers in order of reliability:

| Priority | Layer | Rationale |
|----------|-------|-----------|
| 1 | Arweave | Permanent, content-addressed |
| 2 | IPFS (pinned) | Content-addressed, widely available |
| 3 | GCS/S3 | Centralized but fast |

**Multiple Primary Validators**:

For applications with redundant Primaries (high availability), each Primary publishes to its own storage. The storageRef contains all publication URIs:

```
storageRef: "ar://primary1_tx|ar://primary2_tx|ar://primary3_tx"
```

Witness Validators fetch from any available source and verify content matches the metadataHash.

**Handling Storage Propagation Delays**:

Storage layers have different propagation times:
- GCS/S3: Instant
- IPFS: Seconds (with pinning) to minutes
- Arweave: 5-10 minutes for confirmation

Witness Validators should implement retry logic with backoff when storageRef is not yet available.

### 4.5 Validation Levels

| Level | Enforced By | What's Validated |
|-------|-------------|------------------|
| **Protocol** | All Validators | Replay protection, nonce, timestamp, appId |
| **Calldata** | All Validators | ABI encoding matches messageType signature |
| **Metadata** | All Validators | Schema compliance, required evidence fields |
| **Invariants** | All Validators | Business rules from metadata (limits, balances) |
| **Re-derivation** | Witness Validators | Independent verification of verifiable claims |
| **Bridge** | Smart Contract | Message type registration, signature threshold |

### 4.6 Primary Validator HTTP API

```yaml
# Submit a message for validation and signing
POST /messages
  Headers:
    Authorization: mTLS client cert or API key
  Request:
    messageType: string      # ABI signature
    calldata: bytes          # ABI-encoded function parameters
    metadata: object         # Evidence for validators
    nonce: uint64            # Application nonce
    timestamp: uint64        # Unix timestamp
    appId: bytes32           # Application ID
    value?: uint256          # Native token amount (optional)
  Response:
    status: "accepted" | "rejected"
    messageId?: bytes32
    signature?: bytes
    storageReference?: string
    error?: object

# Check status of a submitted message
GET /messages/{messageId}
  Response:
    id: bytes32
    status: "pending" | "signed" | "published" | "submitted" | "executed"
    signatures: address[]
    storageReference?: string
    bridgeTxHash?: bytes32

# Get validator health and sync status
GET /health
  Response:
    healthy: boolean
    synced: boolean          # Synced with Bridge contract
    bridgeConnection: boolean
    daConnection: boolean
    lastProcessedNonce: map[appId => nonce]

# Get schema for a message type
GET /schemas/{messageType}
  Response:
    schema: object           # JSON Schema
    hash: bytes32            # Schema hash (for verification)
    source: "chain" | "ipfs" | "arweave"
    cached: boolean
```

### 4.7 Error Handling

Validators return structured errors for rejected messages:

```json
{
  "status": "rejected",
  "error": {
    "code": "SCHEMA_VALIDATION_FAILED",
    "message": "Missing required field: recipient",
    "details": {
      "field": "recipient",
      "constraint": "required",
      "schema": "mint(address,uint256)"
    }
  }
}
```

Error codes:

- `REPLAY_DETECTED` - Message ID already processed
- `INVALID_NONCE` - Nonce not greater than last seen
- `TIMESTAMP_EXPIRED` - Timestamp outside acceptable window
- `APP_NOT_AUTHORIZED` - Application ID not registered or inactive
- `AUTH_FAILED` - mTLS certificate or API key invalid
- `MESSAGE_TYPE_NOT_REGISTERED` - Unknown message type
- `MESSAGE_TYPE_DISABLED` - Message type currently disabled
- `CALLDATA_INVALID` - Calldata doesn't match ABI signature
- `CALLDATA_DECODE_FAILED` - Cannot decode calldata with messageType ABI
- `SCHEMA_VALIDATION_FAILED` - Metadata doesn't match schema
- `INVARIANT_VIOLATED` - Metadata invariant check failed
- `METADATA_REDERIVATION_FAILED` - Witness validator couldn't verify metadata claim
- `RATE_LIMIT_EXCEEDED` - Too many messages from this app
- `CUSTOM_RULE_FAILED` - Custom validation rule failed
- `STORAGE_PUBLISH_FAILED` - Failed to publish to storage layer
- `BRIDGE_SUBMIT_FAILED` - Failed to submit signature to Bridge

---

## 5. Schema Registration

### 5.1 Overview

The Bridge maintains a registry of allowed message types. Each message type has:

- A target contract to call (calldata is executed here)
- A JSON Schema defining required **metadata** fields (evidence for validators)
- An enabled/disabled state

**Important distinction**: The schema defines metadata requirements, NOT calldata. Calldata follows the standard Ethereum ABI encoding based on the function signature. Metadata is the additional evidence applications provide to convince validators.

New message types are added via Bridge admin functions. Validators fetch and cache schemas to validate incoming metadata.

### 5.2 Message Type Registry

The Bridge stores message type configurations:

```solidity
struct MessageTypeConfig {
    // Computed from messageType string
    bytes4 selector;

    // Contract to call when executing this message type
    address target;

    // Hash of the metadata JSON Schema (keccak256)
    // NOTE: This is for metadata validation, not calldata
    bytes32 schemaHash;

    // URI to fetch full schema (IPFS, Arweave, or empty for on-chain)
    string schemaUri;

    // Whether this message type is currently active
    bool enabled;

    // When this message type was registered
    uint256 createdAt;

    // When the schema was last updated
    uint256 updatedAt;
}

// Registry: "mint(address,uint256)" => MessageTypeConfig
mapping(string => MessageTypeConfig) public messageTypes;
```

### 5.3 JSON Schema Format

Schemas define the **metadata** (validator evidence) required for each message type. Remember: calldata follows standard ABI encoding and doesn't need a schema.

**Example: ERC20 Mint Metadata Schema**

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "mint(address,uint256)",
  "title": "ERC20 Mint Metadata",
  "description": "Evidence required to validate a mint request",
  "type": "object",
  "required": ["reason", "sourceChain", "sourceTxHash"],
  "properties": {
    "reason": {
      "type": "string",
      "enum": ["user_deposit", "reward", "airdrop", "migration"],
      "description": "Why this mint is being requested"
    },
    "sourceChain": {
      "type": "string",
      "description": "Chain where the triggering event occurred"
    },
    "sourceTxHash": {
      "type": "string",
      "pattern": "^0x[a-fA-F0-9]{64}$",
      "description": "Transaction hash on source chain (verifiable by witness validators)"
    },
    "depositAmount": {
      "type": "string",
      "pattern": "^[0-9]+$",
      "description": "Amount deposited on source chain"
    },
    "userTotalMinted": {
      "type": "string",
      "pattern": "^[0-9]+$",
      "description": "User's total minted balance (invariant check)"
    },
    "contractTotalSupply": {
      "type": "string",
      "pattern": "^[0-9]+$",
      "description": "Contract's current total supply (invariant check)"
    },
    "maxSupply": {
      "type": "string",
      "pattern": "^[0-9]+$",
      "description": "Maximum allowed supply (invariant limit)"
    }
  },
  "additionalProperties": true
}
```

Note: The actual mint parameters (recipient address, amount) are in the **calldata**, not the metadata. The metadata provides context for why validators should approve this mint.

Key points:

- `required` specifies mandatory evidence fields
- `additionalProperties: true` allows extra metadata without schema changes
- Invariant values (limits, balances) are included for validation
- Verifiable fields (sourceTxHash) can be re-derived by Witness Validators

### 5.4 Schema Storage Options

**Option A: On-Chain (Small Schemas)**

For simple schemas, store the full JSON on-chain:

```solidity
mapping(bytes32 => string) public schemas;
// schemaHash => JSON Schema string

function getSchema(string calldata messageType)
    external view returns (string memory)
{
    bytes32 hash = messageTypes[messageType].schemaHash;
    return schemas[hash];
}
```

Pros: No external dependencies, always available
Cons: Gas costs for large schemas

**Option B: IPFS/Arweave (Large Schemas)**

Store schema hash on-chain, full schema on decentralized storage:

```solidity
// On-chain: just the hash and URI
schemaHash: 0x1234...
schemaUri: "ipfs://QmXyz..." // or "ar://abc123..."

// Validators fetch from URI and verify hash matches
```

Pros: Low gas costs, arbitrary schema size
Cons: Requires fetching from external source

**Option C: Hybrid (Recommended)**

- Small/critical schemas stored on-chain
- Large schemas stored on IPFS/Arweave with hash verification
- Validators cache all schemas locally
- Cache invalidated when `updatedAt` changes

### 5.5 Schema Registration API

```solidity
interface IMessageTypeRegistry {
    // Register a new message type
    function registerMessageType(
        string calldata messageType,  // e.g., "mint(address,uint256)"
        address target,               // Contract to call
        bytes32 schemaHash,           // keccak256 of JSON Schema
        string calldata schemaUri     // Where to fetch schema
    ) external;

    // Update schema for existing message type
    function updateSchema(
        string calldata messageType,
        bytes32 newSchemaHash,
        string calldata newSchemaUri
    ) external;

    // Enable or disable a message type
    function setEnabled(
        string calldata messageType,
        bool enabled
    ) external;

    // Update target contract
    function setTarget(
        string calldata messageType,
        address newTarget
    ) external;

    // Query functions
    function isRegistered(string calldata messageType)
        external view returns (bool);

    function isEnabled(string calldata messageType)
        external view returns (bool);

    function getConfig(string calldata messageType)
        external view returns (MessageTypeConfig memory);
}
```

### 5.6 Schema Versioning

**Key insight**: Function signatures (messageType) are stable. `transfer(address,uint256)` doesn't change. What evolves is the metadata schema as applications add new invariants or context for validators.

When schemas change:

1. **Adding optional fields**: No schema update needed (`additionalProperties: true`)
2. **Adding required fields**: New schema version required
3. **Removing fields**: Careful - may break existing applications
4. **Changing types**: New schema version required

For breaking changes:

1. Register new message type (e.g., `mintV2(address,uint256,uint256)`)
2. Migrate applications to new type
3. Disable old message type

### 5.7 Validator Schema Caching

**The Bridge is the canonical source of truth** for message types and schemas. Validators derive schema information by indexing Bridge events.

Validators cache schemas for performance:

```
1. On startup: Fetch all registered message types from Bridge
2. For each type: Fetch schema from URI (or chain)
3. Verify: keccak256(schema) == schemaHash
4. Cache: Store schema locally with hash
5. On use: Validate metadata against cached schema
6. On update: Watch for SchemaUpdated events, refresh cache
```

Cache invalidation events:

```solidity
event MessageTypeRegistered(string indexed messageType, address target, bytes32 schemaHash);
event SchemaUpdated(string indexed messageType, bytes32 oldHash, bytes32 newHash);
event MessageTypeEnabled(string indexed messageType, bool enabled);
```

---

## 6. Bridge Contract Interface

### 6.1 Overview

The Bridge smart contract is the trust anchor of the system. It:

- Maintains the message type registry
- Collects and aggregates validator signatures
- Enforces signature thresholds
- Executes validated messages on target contracts
- Runs modular pre/post-execution checks

### 6.2 Core Interface

```solidity
interface IMessageBridge {
    // ==================== Message Initialization ====================
    // Only the Primary Validator can initialize messages

    /**
     * Initialize a new message on the Bridge
     * @param messageId Unique message identifier (hash of content)
     * @param messageType ABI signature (e.g., "mint(address,uint256)")
     * @param calldata_ ABI-encoded function parameters
     * @param metadataHash Hash of the metadata JSON (for verification)
     * @param storageRef Reference to metadata in storage layer (e.g., "ar://...", "ipfs://...")
     * @param nonce Application nonce
     * @param timestamp Message timestamp
     * @param appId Application identifier
     * @dev Only callable by registered Primary Validator for this appId
     */
    function initializeMessage(
        bytes32 messageId,
        string calldata messageType,
        bytes calldata calldata_,
        bytes32 metadataHash,
        string calldata storageRef,
        uint64 nonce,
        uint64 timestamp,
        bytes32 appId
    ) external payable;

    /**
     * Initialize and sign in one transaction (convenience for Primary)
     * @dev Equivalent to initializeMessage() + signMessage()
     */
    function initializeAndSign(
        bytes32 messageId,
        string calldata messageType,
        bytes calldata calldata_,
        bytes32 metadataHash,
        string calldata storageRef,
        uint64 nonce,
        uint64 timestamp,
        bytes32 appId,
        bytes calldata signature
    ) external payable;

    // ==================== Signature Submission ====================

    /**
     * Submit a validator signature for an initialized message
     * @param messageId Message to sign (must already be initialized)
     * @param signature Validator's EIP-712 signature
     */
    function signMessage(bytes32 messageId, bytes calldata signature) external;

    /**
     * Reject a message with reason (for audit trail)
     * @param messageId Message being rejected
     * @param reasonHash Hash of rejection reason JSON
     * @param reasonRef Storage reference to full rejection reason
     * @dev Validators call this to publicly log why they refused to sign
     */
    function rejectMessage(
        bytes32 messageId,
        bytes32 reasonHash,
        string calldata reasonRef
    ) external;

    // ==================== Execution ====================

    /**
     * Execute a message after signature threshold is met
     * @param messageId Message to execute
     * @dev Can be called by anyone (relayer, validator, etc.)
     * @dev No built-in incentive; reimbursement handled out-of-band
     */
    function executeMessage(bytes32 messageId) external;

    // ==================== Validator Management ====================
    // Note: Validators must complete TEE bootstrapping before being added
    // See Section 6.9 for bootstrapping details

    /**
     * Register Primary Validator for an application
     * @param appId Application identifier
     * @param validator Address derived from TEE-attested signing key
     * @param attestation TEE attestation proving key was generated in enclave
     */
    function setPrimaryValidator(
        bytes32 appId,
        address validator,
        bytes calldata attestation
    ) external;

    /**
     * Add a Witness Validator to the set
     * @param validator Address derived from TEE-attested signing key
     * @param attestation TEE attestation proving key was generated in enclave
     */
    function addWitnessValidator(address validator, bytes calldata attestation) external;

    /**
     * Remove a validator from the set
     */
    function removeValidator(address validator) external;

    /**
     * Set the signature threshold (M of N)
     */
    function setSignatureThreshold(uint256 threshold) external;

    // ==================== Queries ====================

    function getMessageState(bytes32 messageId) external view returns (MessageState memory);
    function getSignatureCount(bytes32 messageId) external view returns (uint256);
    function getRejectionCount(bytes32 messageId) external view returns (uint256);
    function hasValidatorSigned(bytes32 messageId, address validator) external view returns (bool);
    function hasValidatorRejected(bytes32 messageId, address validator) external view returns (bool);
    function isMessageExecuted(bytes32 messageId) external view returns (bool);
    function getPrimaryValidator(bytes32 appId) external view returns (address);
    function getWitnessValidators() external view returns (address[] memory);
    function getSignatureThreshold() external view returns (uint256);
}
```

### 6.3 Message State Machine

Messages progress through defined stages:

```
  initializeMessage() or
  initializeAndSign()
         │
         ▼
    ┌──────────┐     signMessage()      ┌─────────────┐
    │ PENDING  │ ──────────────────────▶│ READY       │
    │          │     threshold met      │ (threshold) │
    └──────────┘                        └─────────────┘
         │                                    │
         ├─ rejectMessage()                   │ executeMessage()
         │  (logs rejection)                  ▼
         │                              ┌─────────────┐
         ├─ expireMessage()             │ PRE_EXEC    │
         │  (after 24h)                 │ (modules)   │
         ▼                              └─────────────┘
    ┌──────────┐                              │
    │ EXPIRED  │                              │ pre-checks pass
    │(terminal)│                              ▼
    └──────────┘                        ┌─────────────┐
                                        │ EXECUTING   │
                                        │ (target)    │
                                        └─────────────┘
                                              │
                                         success │ failure
                                              ▼      ▼
                                   ┌─────────────┐  ┌──────────┐
                                   │ POST_EXEC   │  │ FAILED   │
                                   │ (modules)   │  │(terminal)│
                                   └─────────────┘  └──────────┘
                                         │
                                         │ post-checks pass
                                         ▼
                                   ┌─────────────┐
                                   │ COMPLETED   │
                                   │ (terminal)  │
                                   └─────────────┘

Note: rejectMessage() logs a rejection but doesn't block execution.
      A message can have rejections AND still reach threshold.
      EXPIRED and FAILED are terminal states (nonce consumed).
```

```solidity
enum MessageStage {
    NotInitialized, // Message doesn't exist
    Pending,        // Initialized, collecting signatures
    Ready,          // Threshold met, awaiting execution
    PreExecution,   // Running pre-execution modules
    Executing,      // Calling target contract
    PostExecution,  // Running post-execution modules
    Completed,      // Successfully executed (terminal, nonce consumed)
    Failed,         // Execution failed (terminal, nonce consumed)
    Expired         // Threshold not reached in time (terminal, nonce consumed)
}

struct MessageState {
    MessageStage stage;
    string messageType;
    bytes calldata_;          // ABI-encoded function parameters
    bytes32 metadataHash;     // Hash of metadata JSON (for verification)
    string storageRef;        // Reference to full metadata in storage layer
    uint256 value;
    uint64 nonce;
    uint64 timestamp;
    bytes32 appId;
    address primaryValidator; // Who initialized this message
    uint256 signaturesCollected;
    uint256 rejectionsCollected;
}

struct Rejection {
    address validator;
    bytes32 reasonHash;
    string reasonRef;         // Storage reference to rejection reason
    uint64 timestamp;
}

// Rejections stored separately
mapping(bytes32 => Rejection[]) public messageRejections;
```

### 6.4 Signature Verification

Validators sign the message hash using secp256k1:

```solidity
function _verifySignature(
    bytes32 messageId,
    bytes calldata signature
) internal view returns (address) {
    // Construct EIP-712 typed data hash
    bytes32 structHash = keccak256(abi.encode(
        MESSAGE_TYPEHASH,
        messageId,
        messageStates[messageId].messageType,
        keccak256(messageStates[messageId].payload),
        messageStates[messageId].nonce,
        messageStates[messageId].timestamp,
        messageStates[messageId].appId
    ));

    bytes32 digest = keccak256(abi.encodePacked(
        "\x19\x01",
        DOMAIN_SEPARATOR,
        structHash
    ));

    // Recover signer from signature
    address signer = ECDSA.recover(digest, signature);

    // Verify signer is a registered validator
    require(isValidator[signer], "Not a validator");

    return signer;
}
```

### 6.5 Execution Logic

```solidity
function executeMessage(bytes32 messageId) external nonReentrant {
    MessageState storage state = messageStates[messageId];

    require(state.stage == MessageStage.Ready, "Not ready for execution");

    // Get message type config
    MessageTypeConfig memory config = messageTypes[state.messageType];
    require(config.enabled, "Message type disabled");

    // Stage: Pre-execution modules
    state.stage = MessageStage.PreExecution;
    _runPreModules(messageId, state);

    // Stage: Execute
    state.stage = MessageStage.Executing;

    (bool success, bytes memory returnData) = config.target.call{value: state.value}(
        state.payload
    );

    if (!success) {
        state.stage = MessageStage.Failed;
        emit MessageFailed(messageId, "Execution reverted", returnData);
        // Note: Failed is terminal. Nonce is consumed. Application must retry with new nonce.
        return;
    }

    // Stage: Post-execution modules
    state.stage = MessageStage.PostExecution;
    _runPostModules(messageId, state);

    // Complete
    state.stage = MessageStage.Completed;
    emit MessageExecuted(messageId, state.messageType, config.target);
}
```

### 6.6 Module System

Pre and post-execution modules provide extensible validation.

**Module Scope**:

Modules can be configured at two levels:

| Scope | Applies To | Use Case |
|-------|------------|----------|
| **Global** | All message types | Rate limiting, monitoring |
| **Per-Message-Type** | Specific message types | Amount thresholds for transfers, timelocks for withdrawals |

**Module Registration**:

```solidity
struct ModuleConfig {
    address module;
    bool preExecution;      // Run before execution
    bool postExecution;     // Run after execution
    bool global;            // Apply to all message types
    string[] messageTypes;  // If not global, which message types
}

// Storage
mapping(address => ModuleConfig) public modules;
mapping(string => address[]) public messageTypeModules; // messageType => module addresses

// Register a global module
function addGlobalModule(
    address module,
    bool preExecution,
    bool postExecution
) external;

// Register a module for specific message types
function addModuleForTypes(
    address module,
    bool preExecution,
    bool postExecution,
    string[] calldata messageTypes
) external;

// Remove module from a message type
function removeModuleFromType(address module, string calldata messageType) external;
```

**Module Execution Logic**:

```solidity
function _runPreModules(bytes32 messageId, MessageState storage state) internal {
    // Run global pre-modules
    for (uint i = 0; i < globalPreModules.length; i++) {
        (bool pass, string memory reason) = IModule(globalPreModules[i]).check(messageId, true);
        require(pass, reason);
    }

    // Run message-type-specific pre-modules
    address[] storage typeModules = messageTypeModules[state.messageType];
    for (uint i = 0; i < typeModules.length; i++) {
        ModuleConfig storage config = modules[typeModules[i]];
        if (config.preExecution) {
            (bool pass, string memory reason) = IModule(typeModules[i]).check(messageId, true);
            require(pass, reason);
        }
    }
}
```

**Module Interface**:

```solidity
interface IModule {
    /**
     * Check if a message passes this module's validation
     * @param messageId The message being validated
     * @param isPreExecution True for pre-execution, false for post-execution
     * @return pass True if validation passes
     * @return reason Explanation if validation fails
     */
    function check(bytes32 messageId, bool isPreExecution)
        external view returns (bool pass, string memory reason);
}
```

**Common Module Types**:

```solidity
// Global modules (apply to all messages)
contract RateLimitModule is IModule {
    // Limit messages per time window per application
}

contract MonitoringModule is IModule {
    // Log all messages for external monitoring (always passes)
}

// Per-message-type modules
contract AmountThresholdModule is IModule {
    // For transfer/withdraw: flag or delay large value transfers
    // Configured per message type with different thresholds
}

contract AllowlistModule is IModule {
    // For mint/transfer: restrict to known addresses
}

contract TimelockModule is IModule {
    // For withdraw/upgrade: delay high-value or sensitive operations
}

contract InvariantModule is IModule {
    // For mint: verify post-execution invariants (e.g., supply cap)
}
```

**Example Configuration**:

```solidity
// Global rate limit for all messages
addGlobalModule(rateLimitModule, true, false);

// Amount threshold only for transfers and withdrawals
addModuleForTypes(
    amountThresholdModule,
    true,   // pre-execution
    false,  // no post-execution
    ["transfer(address,uint256)", "withdraw(address,uint256)"]
);

// Timelock only for withdrawals over certain amount
addModuleForTypes(timelockModule, true, false, ["withdraw(address,uint256)"]);

// Supply cap check after mints
addModuleForTypes(invariantModule, false, true, ["mint(address,uint256)"]);
```

### 6.7 Events

These are enshrined Bridge events (not module events). The bridge-wide signature threshold can be extended for specific sensitive transactions via pre-execution modules.

```solidity
// Message lifecycle events
event MessageInitialized(
    bytes32 indexed messageId,
    bytes32 indexed appId,
    address primaryValidator,
    string messageType,
    string storageRef
);
event SignatureSubmitted(bytes32 indexed messageId, address indexed validator, uint256 count);
event MessageRejected(
    bytes32 indexed messageId,
    address indexed validator,
    bytes32 reasonHash,
    string reasonRef
);
event ThresholdReached(bytes32 indexed messageId, uint256 signatures);

// Execution events
event MessageExecuted(bytes32 indexed messageId, string messageType, address target);
event MessageFailed(bytes32 indexed messageId, string reason, bytes data);

// Validator events
event PrimaryValidatorSet(bytes32 indexed appId, address indexed validator, bytes attestation);
event WitnessValidatorAdded(address indexed validator, bytes attestation);
event ValidatorRemoved(address indexed validator);
event ThresholdUpdated(uint256 oldThreshold, uint256 newThreshold);

// Module events
event ModuleAdded(address indexed module, bool preExecution, bool postExecution);
event ModuleRemoved(address indexed module);

// Registry events
event MessageTypeRegistered(string indexed messageType, address target, bytes32 schemaHash);
event MessageTypeUpdated(string indexed messageType, bytes32 oldSchemaHash, bytes32 newSchemaHash);
event MessageTypeEnabled(string indexed messageType, bool enabled);
```

### 6.8 Access Control

```solidity
// Roles
bytes32 public constant ADMIN_ROLE = keccak256("ADMIN_ROLE");
bytes32 public constant REGISTRY_ROLE = keccak256("REGISTRY_ROLE");
bytes32 public constant VALIDATOR_MANAGER_ROLE = keccak256("VALIDATOR_MANAGER_ROLE");

// Permissions
// - ADMIN_ROLE: Can grant/revoke all roles
// - REGISTRY_ROLE: Can register/update message types
// - VALIDATOR_MANAGER_ROLE: Can add/remove validators, set threshold

// Timelock recommended for: threshold changes, validator removal, message type disabling
```

### 6.9 TEE Bootstrapping

Validators must complete TEE bootstrapping before they can sign messages. This ensures signing keys are generated and protected within a Trusted Execution Environment.

**Bootstrapping Flow**:

```
┌─────────────────────────────────────────────────────────────────┐
│                    TEE BOOTSTRAPPING                             │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  1. ENCLAVE STARTUP                                              │
│     ├─ Validator starts in TEE (GCP Confidential Space, etc.)   │
│     └─ TEE generates attestation token                           │
│                                                                  │
│  2. KEY GENERATION                                               │
│     ├─ Generate secp256k1 signing key INSIDE enclave            │
│     ├─ Key never leaves enclave memory                           │
│     └─ Derive Ethereum address from public key                   │
│                                                                  │
│  3. ATTESTATION                                                  │
│     ├─ Create attestation binding key to enclave                 │
│     ├─ Attestation includes: code hash, key fingerprint          │
│     └─ Sign attestation with TEE platform key                    │
│                                                                  │
│  4. REGISTRATION                                                 │
│     ├─ Submit to Bridge: addValidator(address, attestation)     │
│     ├─ Bridge verifies attestation (on-chain or via oracle)     │
│     └─ Validator added to signing set                            │
│                                                                  │
│  5. ONGOING OPERATION                                            │
│     ├─ Validator signs messages with enclave-protected key       │
│     └─ No per-message attestation needed (key already attested) │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

**Why TEE Bootstrapping Matters**:

1. **Key Protection**: Signing keys cannot be extracted from the enclave
2. **Code Integrity**: Attestation proves validator runs expected code
3. **One-Time Cost**: Attestation happens once at registration, not per-message
4. **Audit Trail**: Attestation stored on-chain for verification

**Supported TEE Platforms**:

| Platform | Provider | Attestation Format |
|----------|----------|-------------------|
| GCP Confidential Space | Google Cloud | OIDC token with claims |
| AWS Nitro Enclaves | Amazon Web Services | Nitro attestation document |
| Azure Confidential VMs | Microsoft Azure | AMD SEV-SNP attestation |
| Intel SGX | Various | DCAP attestation |

**Example Attestation Verification**:

```solidity
function addValidator(address validator, bytes calldata attestation) external {
    require(hasRole(VALIDATOR_MANAGER_ROLE, msg.sender), "Not authorized");

    // Verify attestation (implementation depends on TEE platform)
    require(_verifyAttestation(validator, attestation), "Invalid attestation");

    // Add to validator set
    isValidator[validator] = true;
    validators.push(validator);

    emit ValidatorAdded(validator, attestation);
}
```

### 6.10 Bridge Upgrades and TEE Versioning

Bridges are designed to be **upgradable** to allow security patches and feature additions without disrupting operations.

**Upgrade Pattern**:

Use the UUPS (Universal Upgradeable Proxy Standard) pattern:

```solidity
contract MessageBridge is UUPSUpgradeable, AccessControlUpgradeable {
    // Implementation logic

    function _authorizeUpgrade(address newImplementation)
        internal
        override
        onlyRole(ADMIN_ROLE)
    {
        // Optional: Require timelock for upgrades
        require(
            block.timestamp >= upgradeProposedAt + UPGRADE_DELAY,
            "Upgrade delay not met"
        );
    }
}
```

**Upgrade Safety**:

| Concern | Mitigation |
|---------|------------|
| Malicious upgrade | Timelock + multisig admin |
| Broken upgrade | Comprehensive test suite, staging deployment |
| State corruption | Storage layout compatibility checks |
| Validator disruption | Graceful version transition period |

**TEE Version Tracking**:

The Bridge tracks the TEE code version for each validator. This enables:
- Enforcing minimum TEE version for signing
- Graceful migration to new validator code
- Audit trail of validator software

```solidity
struct ValidatorInfo {
    address validator;
    bool active;
    uint64 registeredAt;
    bytes32 teeCodeHash;        // Hash of validator code running in TEE
    string teeVersion;          // Semantic version (e.g., "1.2.3")
    uint64 lastAttestationAt;   // When attestation was last verified
}

mapping(address => ValidatorInfo) public validatorInfo;

// Minimum TEE version required for signing
string public minimumTeeVersion;

function setMinimumTeeVersion(string calldata version) external onlyRole(ADMIN_ROLE);

function updateValidatorAttestation(
    address validator,
    bytes calldata attestation,
    string calldata teeVersion
) external {
    // Verify new attestation
    require(_verifyAttestation(validator, attestation), "Invalid attestation");

    // Update validator info
    ValidatorInfo storage info = validatorInfo[validator];
    info.teeCodeHash = _extractCodeHash(attestation);
    info.teeVersion = teeVersion;
    info.lastAttestationAt = uint64(block.timestamp);

    emit ValidatorAttestationUpdated(validator, info.teeCodeHash, teeVersion);
}
```

**Version Enforcement**:

```solidity
function signMessage(bytes32 messageId, bytes calldata signature) external {
    address signer = _verifySignature(messageId, signature);

    // Check validator is active and has valid attestation
    ValidatorInfo storage info = validatorInfo[signer];
    require(info.active, "Validator not active");

    // Check TEE version meets minimum
    require(
        _compareVersions(info.teeVersion, minimumTeeVersion) >= 0,
        "TEE version too old"
    );

    // ... rest of signing logic
}
```

**Upgrade Flow for Validators**:

```
1. New validator version released
2. Admin sets new minimumTeeVersion with grace period
3. Validators upgrade TEE code
4. Validators re-attest with new version
5. After grace period, old versions can't sign
```

**Events**:

```solidity
event BridgeUpgraded(address indexed implementation, uint256 timestamp);
event MinimumTeeVersionUpdated(string oldVersion, string newVersion);
event ValidatorAttestationUpdated(
    address indexed validator,
    bytes32 codeHash,
    string version
);
```

### 6.11 WETH Handling and msg.value

For consistency and security, the Bridge only holds and manages **WETH (Wrapped ETH)**, not native ETH.

**Why WETH Only**:

1. **Consistency**: All value transfers use the same ERC-20 interface
2. **Accounting**: Easier to track balances and approvals
3. **Security**: Prevents reentrancy issues with native ETH transfers
4. **Composability**: Works with DeFi protocols that expect ERC-20

**Execution Flow for Payable Calls**:

When executing a message with `value > 0`:

```solidity
function executeMessage(bytes32 messageId) external nonReentrant {
    MessageState storage state = messageStates[messageId];

    // ... validation ...

    // If message has value, unwrap WETH before calling
    if (state.value > 0) {
        // Bridge holds WETH, unwrap to native ETH for the call
        IWETH(WETH).withdraw(state.value);
    }

    // Execute with native ETH
    (bool success, bytes memory returnData) = config.target.call{value: state.value}(
        state.calldata_
    );

    // ... handle result ...
}
```

**Funding the Bridge**:

Applications that need to execute payable calls must:

1. Wrap ETH to WETH
2. Transfer WETH to the Bridge
3. Include `value` in messages

```solidity
// Application or relayer funds the Bridge
IWETH(WETH).deposit{value: 1 ether}();
IWETH(WETH).transfer(bridgeAddress, 1 ether);
```

**Receiving Value (from executed calls)**:

If target contracts return ETH to the Bridge:

```solidity
// Bridge can re-wrap received ETH
receive() external payable {
    IWETH(WETH).deposit{value: msg.value}();
}
```

**WETH Interface**:

```solidity
interface IWETH {
    function deposit() external payable;
    function withdraw(uint256 amount) external;
    function transfer(address to, uint256 amount) external returns (bool);
    function balanceOf(address account) external view returns (uint256);
}
```

**Relayer Gas Reimbursement**:

Relayers who call `executeMessage()` pay gas costs. Reimbursement is handled out-of-band:
- Direct payment from application
- On-chain reimbursement module (tracks and pays relayers)
- Priority fee on messages

There is no built-in gas reimbursement mechanism in the core protocol.

---

## 7. Trust Model

### 7.1 Core Security Property

**The application does not control Bridge funds.**

This is the fundamental security invariant. Even if an application is fully compromised:

- It cannot request operations outside registered message types
- It cannot bypass schema validation
- It cannot forge validator signatures
- It cannot directly access Bridge funds
- It cannot modify Bridge configuration

The Bridge rules are the trust anchor.

### 7.2 Single Validator Mode

**Use Case**: Non-financial applications, low-value operations, development/testing

```
Application ───▶ Primary Validator ───▶ Storage ───▶ Bridge
                                           │
                                    (no witnesses)
```

In single validator mode, the Primary Validator is the only validator. It receives messages from the application, validates, signs, publishes to storage for audit, and submits to the Bridge.

**Configuration**:

```solidity
signatureThreshold = 1;
validators = [primaryValidatorAddress];
```

**Characteristics**:

- Fastest path to execution (single signature)
- Single point of trust (the Primary Validator)
- Bridge still enforces all rules
- Suitable for: gaming state, social feeds, metadata updates, internal tools

**Trust assumption**: You trust the Primary Validator operator not to sign malicious messages.

### 7.3 Multi-Validator Mode

**Use Case**: Financial applications, high-value operations, production systems

```
                                            ┌───▶ Witness 1 ───┐
                                            │                  │
Application ───▶ Primary Validator ───▶ Storage ───▶ Witness 2 ───┼───▶ Bridge (M-of-N)
                                            │                  │
                                            └───▶ Witness N ───┘
```

In multi-validator mode:
1. **Primary Validator**: Receives messages from application, publishes to storage
2. **Witness Validators**: Read from storage, independently verify, submit signatures

**Configuration**:

```solidity
signatureThreshold = 2;  // 2-of-3
validators = [primaryValidator, witnessValidator1, witnessValidator2];
```

**Characteristics**:

- Byzantine fault tolerant (tolerates (N-M) malicious validators)
- Distributed trust (Witnesses don't trust Primary's HTTP connection)
- Higher latency (multiple independent validations)
- Suitable for: token bridges, DeFi, high-value transfers

**Common configurations**:

- 1 Primary + 1 Witness (2-of-2): Requires agreement
- 1 Primary + 2 Witnesses (2-of-3): Tolerates 1 malicious/offline
- 1 Primary + 4 Witnesses (3-of-5): Tolerates 2 malicious/offline

### 7.4 Trust Boundaries

| Component         | Trust Assumption     | Consequence if Compromised          |
| ----------------- | -------------------- | ----------------------------------- |
| Application       | Minimal              | Can only request allowed operations |
| Single Validator  | Full                 | Can sign any allowed operation      |
| M-of-N Validators | M validators collude | Can sign any allowed operation      |
| Bridge Contract   | Immutable/audited    | System broken (upgrade needed)      |
| Bridge Admin      | Timelock protected   | Can change rules (with delay)       |

### 7.5 Security Gradient

Different applications require different security postures:

| Value at Risk      | Validator Setup           | Additional Measures                      |
| ------------------ | ------------------------- | ---------------------------------------- |
| Low (<$1K)         | Single validator, no TEE  | Basic rate limiting                      |
| Medium ($1K-$100K) | Single validator in TEE   | Rate limits, DA audit                    |
| High ($100K-$1M)   | 2-of-3 validators in TEE  | Amount thresholds, allowlists            |
| Critical (>$1M)    | 3-of-5+ validators in TEE | Timelocks, monitoring, incident response |

### 7.6 Metadata Best Practices

**More metadata = more validator confidence.** Applications that provide rich metadata make it easier for validators to approve messages.

**Minimum Metadata** (always required):
- `reason`: Why this action is being requested
- `timestamp`: When the triggering event occurred

**Recommended Metadata** (for most applications):
- Source chain transaction hash (verifiable by Witness Validators)
- Current state values (balances, supplies)
- Invariant limits (max supply, per-user caps)

**High-Value Metadata** (for financial applications):
- Price at time of action (from oracle or public API)
- User's transaction history summary
- KYC/compliance status
- Application TEE attestation

**Example: Rich Metadata for Cross-Chain Deposit**:

```json
{
  "reason": "user_deposit",
  "sourceChain": "ethereum",
  "sourceTxHash": "0xabc...",
  "sourceBlockNumber": 12345678,
  "sourceTimestamp": 1735084800,
  "depositAmount": "1000000000000000000",
  "exchangeRate": "1.0",
  "priceSource": "chainlink",

  "userDepositCount": 5,
  "userTotalDeposited": "5000000000000000000",
  "userLastDepositTimestamp": 1735000000,

  "contractTotalDeposits": "1000000000000000000000",
  "maxTotalDeposits": "10000000000000000000000",

  "applicationVersion": "1.2.3",
  "applicationTeeAttestation": "eyJ..."
}
```

Witness Validators can independently verify:
- `sourceTxHash` exists on source chain
- `depositAmount` matches event logs
- `exchangeRate` matches public price feeds
- `contractTotalDeposits` matches on-chain query

### 7.7 Application Security Upgrades

Applications can progressively upgrade their security posture:

| Level | Security | Validator Confidence | Use Case |
|-------|----------|---------------------|----------|
| **Basic** | HTTP app, minimal metadata | Low | Internal tools, testing |
| **Logging** | Audit logs, verifiable claims | Medium | Non-financial apps |
| **TEE** | App runs in TEE, attestation in metadata | High | Financial apps |
| **Checkpointing** | Periodic state snapshots | Very High | High-value operations |
| **Full Verification** | Open, deterministic code | Maximum | Critical infrastructure |

The protocol supports all levels; applications choose based on value at risk.

---

## 8. Security Analysis

### 8.0 Role of TEEs

Trusted Execution Environments (TEEs) provide hardware-enforced security guarantees:

**For Validators (Strongly Recommended)**:
- Signing keys generated and stored inside enclave
- Keys cannot be extracted even by the validator operator
- Attestation proves validator runs expected code
- Required for TEE bootstrapping (Section 6.9)

**For Applications (Optional but Recommended for Financial)**:
- Application code runs in protected enclave
- Attestation proves application hasn't been tampered with
- Can include attestation in metadata for validator verification
- Validators can require TEE attestation for high-value operations

**Why TEE for Financial Applications**:

Without TEE, a compromised application operator could:
- Submit fraudulent messages claiming fake deposits
- Manipulate metadata to circumvent invariant checks
- Collude with a malicious Primary Validator

With TEE:
- Application code is attested and cannot be modified
- Validators can verify application integrity via attestation
- Even if operator is malicious, they cannot change the code behavior

### 8.1 Threat Model

**Attackers**:

1. **Malicious Application**: Compromised or malicious application operator
2. **External Attacker**: No access to any system component
3. **Compromised Validator**: One or more validators colluding
4. **Insider Threat**: Bridge admin or operator

**Goals**:

1. Steal funds from Bridge
2. Execute unauthorized operations
3. Manipulate message ordering
4. Cause denial of service

### 8.2 Threat Mitigation

| Threat                     | Attack Vector               | Mitigation                                                    |
| -------------------------- | --------------------------- | ------------------------------------------------------------- |
| Malicious App              | Submit fraudulent messages  | Bridge rules limit possible operations; validators add checks |
| External                   | Forge message signatures    | secp256k1 signatures; validators in TEE                       |
| External                   | Replay old messages         | Unique message IDs; nonce ordering; timestamp freshness       |
| External                   | Tamper with messages        | Signature over full message content                           |
| Compromised Validator (1)  | Sign malicious messages     | M-of-N threshold; other validators must also sign             |
| Compromised Validators (M) | Collude to sign malicious   | Defense in depth with modules; monitoring; timelocks          |
| Insider                    | Modify Bridge rules         | Role-based access; timelocks on admin operations              |
| DoS                        | Flood with invalid messages | Rate limiting; application authorization                      |

### 8.3 Security Properties

**Integrity**: Messages cannot be modified after signing

- Validators sign over complete message hash
- Bridge verifies signature before storing
- Any modification invalidates signature

**Authenticity**: Only registered validators can approve messages

- Validator registry on Bridge contract
- Signature recovery proves signer identity
- TEE attestation proves validator integrity

**Non-repudiation**: All operations are auditable

- DA layer stores all validated messages
- On-chain events for all state changes
- Complete audit trail for disputes

**Authorization**: Bridge controls possible operations

- Message type registry limits allowed operations
- Schema validation ensures correct parameters
- Modules enforce additional constraints

### 8.4 Attack Scenarios

**Scenario 1: Application tries to steal funds**

```
App: "transfer(address,uint256)" with attacker address
Mitigation:
1. Message type must be registered (limited by Bridge admin)
2. Schema requires specific fields (validated by validators)
3. Allowlist module restricts to known addresses
4. Rate limit module prevents draining
5. Amount threshold module flags large transfers
```

**Scenario 2: Replay attack**

```
Attacker: Re-submit old valid message
Mitigation:
1. Message ID is unique (hash of content)
2. Nonce must be strictly increasing
3. Timestamp must be recent
4. Bridge tracks executed message IDs
```

**Scenario 3: Front-running**

```
Attacker: See pending message, submit conflicting transaction
Mitigation:
1. Application can use private mempools
2. Commit-reveal schemes for sensitive operations
3. Nonce ordering prevents reordering
```

**Scenario 4: 1-of-1 Availability Attack**

```
Attack: Primary Validator submits to Bridge but fails to publish to storage
Impact: Message executes, but no audit trail exists; Witness Validators
        cannot verify (in multi-validator mode, this would block execution)

In 1-of-1 mode:
- Bridge is source of truth
- Message executes with only Primary's signature
- No Witness Validators to verify availability
- Audit trail incomplete in storage layer

Mitigation:
1. For 1-of-1 mode, accept this risk as inherent to single-validator trust
2. For multi-validator mode, Witnesses won't sign without storage access
3. Bridge emits storageRef in MessageInitialized event for independent fetching
4. Monitoring: Alert if messages execute without corresponding storage records
5. Application can require multi-validator mode for high-value operations
```

This is a **known vulnerability in 1-of-1 mode**: availability is not checked by other validators. Applications using 1-of-1 mode implicitly trust the Primary Validator for both correctness AND availability.

**Scenario 5: Validator key theft**

```
Attacker: Steal validator private key
Mitigation:
1. Keys stored in TEE (hardware protected)
2. M-of-N threshold requires multiple keys
3. Key rotation procedures
4. Monitoring for anomalous signing patterns
```

### 8.5 Security Recommendations

**For Bridge Operators** (typically the application developer):

1. Use multisig for admin operations (avoid unilateral control)
2. Implement timelocks for sensitive changes (threshold, validator removal)
3. Regular security audits of contracts
4. Monitoring and alerting for anomalies
5. Incident response procedures
6. **Consider a Security Council**: For high-value bridges, delegate admin to a diverse council to prevent single points of failure

**For Validator Operators**:

1. Run validators in TEE (mandatory for multi-validator mode)
2. Implement key rotation procedures
3. Monitor signing patterns for anomalies
4. Rate limit per application
5. Geographic distribution for resilience
6. **Implement custom validation logic** based on available metadata:
   - Verify `sourceTxHash` on source chain
   - Check prices against public oracles
   - Query on-chain state for invariant values
   - Flag messages with unusual patterns

**For Application Developers**:

1. Use appropriate validator configuration for value at risk
2. Implement nonce management correctly
3. Add application-level rate limits
4. Log all message submissions
5. Monitor for failed validations
6. **Provide rich metadata** to increase validator confidence (Section 7.6)

---

## 9. Storage and Data Availability

### 9.1 Purpose

Storage and DA layers serve different purposes:

**Storage Layer** (Required):
- Long-term archival of all messages
- Schema storage and distribution
- Witness Validators read messages from here
- Must have strong durability guarantees

**DA Layer** (Optional):
- Short-term data availability guarantees
- Useful for high-throughput applications
- Provides consensus-level availability (if using Celestia/EigenDA)

### 9.2 Publication Format

Messages published to storage include:

```typescript
interface StorageRecord {
  // The original message
  message: {
    id: bytes32;
    messageType: string;
    calldata: bytes;      // ABI-encoded function parameters
    metadata: object;     // Validator evidence
    nonce: uint64;
    timestamp: uint64;
    appId: bytes32;
    value?: uint256;
  };

  // Primary Validator signature
  primarySignature: {
    validator: address;
    signature: bytes;
    signedAt: uint64;
  };

  // Publication metadata
  publication: {
    publishedBy: address;
    publishedAt: uint64;
    storageLayer: string;
    storageReference: string;
  };
}
```

### 9.3 Publication Timing

**Always publish BEFORE Bridge submission.** Witness Validators must be able to read messages from storage before they sign.

**Batched Publication** (Recommended for most use cases):
- Primary Validator buffers messages
- Publishes batch every N seconds or M messages
- Lower storage costs, slight delay
- Suitable for: most applications

```
Application → Primary → [Sign] → [Buffer] → [Batch Publish] → [Submit to Bridge]
                                                  ↓
                                          Witness reads from storage
```

**Immediate Publication** (For latency-sensitive applications):
- Publish each message immediately after signing
- Higher storage costs, lowest latency
- Suitable for: time-critical operations

```
Application → Primary → [Sign] → [Publish] → [Submit to Bridge]
                                     ↓
                            Witness reads immediately
```

### 9.4 Storage and DA Options

**Storage Layers** (for archival - at least one required):

| Layer | Durability | Cost | Latency | Recommendation |
|-------|------------|------|---------|----------------|
| **Arweave** | Permanent | Medium | ~5min | **Primary** - permanent archival |
| **IPFS + Pinning** | Long-term | Low | Instant | Good alternative with pinning service |
| **GCS/S3** | Centralized | Low | Instant | Internal/development only |

**DA Layers** (for short-term availability - optional):

| Layer | Availability | Cost | Latency | Use Case |
|-------|--------------|------|---------|----------|
| **Celestia** | ~2 weeks | Low | ~12s | High-volume, needs consensus availability |
| **EigenDA** | ~2 weeks | Low | ~12s | Ethereum-aligned availability |

**Recommended Configuration**:
- **Storage** (required): Arweave for permanent archival, IPFS for schema distribution
- **DA** (optional): Celestia if you need consensus-level availability guarantees

### 9.5 Validator Storage Configuration

```yaml
# Primary Validator storage configuration
storage:
  # Primary storage layer (required)
  primary: arweave

  # Fallback if primary fails
  fallback: ipfs

  # Publication mode
  mode: batched  # batched (recommended) | immediate

  # Batching settings (if mode: batched)
  batch:
    max_messages: 100
    max_delay_seconds: 30

  # Layer-specific config
  arweave:
    gateway: "https://arweave.net"
    wallet_path: "/path/to/wallet.json"
```

### 9.6 DA Reference Tracking

Validators track DA references for queries:

```sql
CREATE TABLE da_publications (
    message_id BYTES32 PRIMARY KEY,
    da_layer TEXT NOT NULL,
    da_reference TEXT NOT NULL,
    published_at INTEGER NOT NULL,
    confirmed BOOLEAN DEFAULT FALSE,
    INDEX idx_published_at (published_at)
);
```

Query API:

```
GET /messages/{messageId}/da
Response: {
  layer: "celestia",
  reference: "celestia://namespace/height/index",
  publishedAt: 1735084800,
  confirmed: true
}
```

---

## 10. Implementation Phases

### Phase 1: Core Infrastructure (MVP)

**Goal**: Minimal working system with single validator

**Deliverables**:

1. **Message Types and Schemas** (`synddb-shared`)
   - New message structures (Message, SignedMessage)
   - JSON Schema validation integration
   - Remove changeset-related types

2. **Validator Service** (`synddb-validator`)
   - New operating mode: `--mode message-passing`
   - HTTP API: `POST /messages`, `GET /messages/{id}`, `GET /health`
   - Basic validation: type registration, schema, nonce
   - Signing with existing key management

3. **Bridge Contract** (`contracts/`)
   - Message type registry
   - Single-signature execution path
   - Basic events and state tracking

4. **Remove Legacy**
   - Deprecate changeset code paths
   - Remove sequencer dependency from validator

**Success Criteria**:

- Application can POST message to validator
- Validator validates and signs
- Message executes on Bridge

### Phase 2: Multi-Validator Support

**Goal**: Production-ready multi-signature support

**Deliverables**:

1. **On-Chain Signature Aggregation**
   - `signMessage()` for individual signatures
   - Threshold tracking per message
   - Signature deduplication

2. **Validator Coordination**
   - Message status API across validators
   - Relayer pattern for batched submission
   - Signature expiration handling

3. **Full Bridge Contract**
   - Complete IMessageBridge implementation
   - EIP-712 signature verification
   - Module system integration

**Success Criteria**:

- 2-of-3 validators can approve message
- Threshold enforcement works correctly
- Validators operate independently

### Phase 3: Schema Management

**Goal**: Flexible schema storage and validation

**Deliverables**:

1. **IPFS/Arweave Schema Storage**
   - Schema upload tooling
   - Validator schema caching
   - Cache invalidation on updates

2. **Schema Versioning**
   - Version tracking per message type
   - Migration documentation
   - Backward compatibility handling

3. **Admin Tools**
   - CLI for message type registration
   - Schema validation testing
   - Registry inspection

**Success Criteria**:

- Schemas can be stored on IPFS/Arweave
- Validators cache and verify schemas
- Admin can register new message types

### Phase 4: DA Integration

**Goal**: Audit trail for all messages

**Deliverables**:

1. **DA Publishers**
   - Celestia publisher
   - Arweave publisher
   - IPFS publisher (development)

2. **Publication Modes**
   - Immediate publication
   - Batched publication
   - Configurable per validator

3. **Query and Verification**
   - DA reference tracking
   - Message retrieval API
   - Verification tooling

**Success Criteria**:

- All messages published to DA
- Messages can be retrieved and verified
- Publication is reliable and monitored

### Phase 5: Production Hardening

**Goal**: Production-ready security and operations

**Deliverables**:

1. **Additional Modules**
   - Rate limiting module
   - Amount threshold module
   - Time delay module
   - Allowlist module

2. **Monitoring and Alerting**
   - Prometheus metrics
   - Alert rules for anomalies
   - Dashboard templates

3. **Documentation**
   - Integration guide
   - Security best practices
   - Deployment runbooks
   - Troubleshooting guide

**Success Criteria**:

- Modules deployed and tested
- Monitoring operational
- Documentation complete

---

## 11. Implementation Notes

### 11.1 Clean Slate Implementation

This specification describes a **new system**, not a migration from the SQLite replication architecture. SyndDB is not yet deployed to production, so there are no backward compatibility requirements.

**What's new**:
- Message passing replaces SQLite changeset capture
- Validators replace the sequencer concept
- Primary/Witness validator model for multi-validator security
- Metadata-based validation instead of SQL replay

**Crate changes**:
| Crate | Status |
|-------|--------|
| `synddb-client` | Deprecated (no longer needed) |
| `synddb-sequencer` | Deprecated (validators handle this) |
| `synddb-validator` | Evolves to support message-passing mode |
| `synddb-shared` | New message types for this spec |

---

## 12. Appendix

### A. Example Integration

**Minimal Application Integration**:

```python
import requests
import time
from eth_abi import encode

VALIDATOR_URL = "https://validator.example.com"
APP_ID = "0x" + "00" * 31 + "01"

nonce = 0

def encode_calldata(function_sig: str, *args) -> str:
    """Encode function call to hex calldata."""
    from web3 import Web3
    selector = Web3.keccak(text=function_sig)[:4].hex()
    # Parse types from signature
    types = function_sig.split("(")[1].rstrip(")").split(",")
    encoded_args = encode(types, args).hex()
    return "0x" + selector[2:] + encoded_args

def submit_message(message_type: str, calldata: str, metadata: dict, value: int = 0):
    """Submit a message to the Primary Validator."""
    global nonce
    nonce += 1

    message = {
        "messageType": message_type,
        "calldata": calldata,           # ABI-encoded function parameters
        "metadata": metadata,           # Evidence for validators
        "nonce": nonce,
        "timestamp": int(time.time()),
        "appId": APP_ID,
        "value": str(value) if value else None
    }

    response = requests.post(
        f"{VALIDATOR_URL}/messages",
        json=message
    )

    result = response.json()
    if result["status"] == "accepted":
        return result["messageId"]
    else:
        raise Exception(result["error"])

# Example: Mint tokens
recipient = "0x742d35Cc6634C0532925a3b844Bc454e4438f44e"
amount = 1000000000000000000  # 1 token in wei

message_id = submit_message(
    "mint(address,uint256)",
    encode_calldata("mint(address,uint256)", recipient, amount),
    {
        "recipient": "0x742d35Cc6634C0532925a3b844Bc454e4438f44e",
        "amount": "1000000000000000000",
        "reason": "user_deposit"
    }
)
print(f"Submitted: {message_id}")
```

### B. JSON Schema Examples

These schemas define **metadata** (validator evidence), NOT function parameters. Function parameters are in the calldata.

**ERC20 Transfer Metadata Schema**:

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "transfer(address,address,uint256)",
  "title": "ERC20 Transfer Metadata",
  "description": "Evidence for validators to approve a transfer",
  "type": "object",
  "required": ["reason"],
  "properties": {
    "reason": {
      "type": "string",
      "enum": ["user_request", "automated_distribution", "migration"],
      "description": "Why this transfer is being requested"
    },
    "sourceReference": {
      "type": "string",
      "description": "Reference to source system (order ID, user ID, etc.)"
    },
    "senderBalance": {
      "type": "string",
      "pattern": "^[0-9]+$",
      "description": "Sender's balance before transfer (invariant check)"
    }
  },
  "additionalProperties": true
}
```

**NFT Batch Mint Metadata Schema**:

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "batchMint(address[],uint256[],string[])",
  "title": "NFT Batch Mint Metadata",
  "description": "Evidence for validators to approve batch mint",
  "type": "object",
  "required": ["reason", "currentTotalSupply", "maxTotalSupply"],
  "properties": {
    "reason": {
      "type": "string",
      "enum": ["airdrop", "presale", "public_mint"],
      "description": "Why this batch mint is being requested"
    },
    "currentTotalSupply": {
      "type": "integer",
      "description": "Current total supply before mint"
    },
    "maxTotalSupply": {
      "type": "integer",
      "description": "Maximum allowed supply (invariant)"
    },
    "batchSize": {
      "type": "integer",
      "minimum": 1,
      "maximum": 100,
      "description": "Number of tokens in this batch"
    },
    "campaignId": {
      "type": "string",
      "description": "Reference to mint campaign"
    }
  },
  "additionalProperties": true
}
```

### C. Glossary

| Term | Definition |
|------|------------|
| **Message Type** | ABI function signature (e.g., `mint(address,uint256)`) |
| **Calldata** | ABI-encoded function parameters executed on-chain |
| **Metadata** | Evidence/context provided to convince validators to sign |
| **Schema** | JSON Schema defining required metadata fields |
| **Primary Validator** | Validator connected to application via HTTP, publishes to storage |
| **Witness Validator** | Validator that reads from storage, verifies independently |
| **Bridge** | Smart contract that aggregates signatures and executes messages |
| **Module** | Pluggable validation for pre/post execution checks |
| **Storage Layer** | Long-term archival (Arweave, IPFS) |
| **DA Layer** | Short-term data availability (Celestia, EigenDA) |
| **Threshold** | Minimum validator signatures required |
| **Nonce** | Monotonically increasing counter for replay protection |
| **TEE** | Trusted Execution Environment (GCP Confidential Space, etc.) |
| **Attestation** | Cryptographic proof of TEE integrity |
