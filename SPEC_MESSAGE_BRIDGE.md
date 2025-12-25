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

### 4.4 Validation Levels

| Level | Enforced By | What's Validated |
|-------|-------------|------------------|
| **Protocol** | All Validators | Replay protection, nonce, timestamp, appId |
| **Calldata** | All Validators | ABI encoding matches messageType signature |
| **Metadata** | All Validators | Schema compliance, required evidence fields |
| **Invariants** | All Validators | Business rules from metadata (limits, balances) |
| **Re-derivation** | Witness Validators | Independent verification of verifiable claims |
| **Bridge** | Smart Contract | Message type registration, signature threshold |

### 4.5 Primary Validator HTTP API

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

### 4.4 Error Handling

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
    // ==================== Message Submission ====================

    /**
     * Submit a message with aggregated signatures (batch submission)
     * @param messageId Unique message identifier
     * @param messageType ABI signature (e.g., "mint(address,uint256)")
     * @param calldata ABI-encoded function parameters
     * @param nonce Application nonce
     * @param timestamp Message timestamp
     * @param appId Application identifier
     * @param signatures Array of validator signatures
     */
    function submitMessage(
        bytes32 messageId,
        string calldata messageType,
        bytes calldata calldata_,
        uint64 nonce,
        uint64 timestamp,
        bytes32 appId,
        bytes[] calldata signatures
    ) external payable;

    /**
     * Submit a single validator signature (on-chain aggregation)
     * @param messageId Message to sign
     * @param signature Validator's signature
     */
    function signMessage(bytes32 messageId, bytes calldata signature) external;

    /**
     * Execute a message after threshold is met
     * @param messageId Message to execute
     */
    function executeMessage(bytes32 messageId) external;

    // ==================== Validator Management ====================
    // Note: Validators must complete TEE bootstrapping before being added
    // See Section 6.9 for bootstrapping details

    /**
     * Add a validator after TEE bootstrapping
     * @param validator Address derived from TEE-attested signing key
     * @param attestation TEE attestation proving key was generated in enclave
     */
    function addValidator(address validator, bytes calldata attestation) external;

    /**
     * Remove a validator from the set
     */
    function removeValidator(address validator) external;

    /**
     * Set the signature threshold (M of N)
     */
    function setSignatureThreshold(uint256 threshold) external;

    // ==================== Queries ====================

    function getSignatureCount(bytes32 messageId) external view returns (uint256);
    function hasValidatorSigned(bytes32 messageId, address validator) external view returns (bool);
    function isMessageExecuted(bytes32 messageId) external view returns (bool);
    function getValidators() external view returns (address[] memory);
    function getSignatureThreshold() external view returns (uint256);
}
```

### 6.3 Message State Machine

Messages progress through defined stages:

```
                    signMessage()
                         │
         ┌───────────────┴───────────────┐
         │                               │
         ▼                               ▼
    ┌─────────┐                    ┌─────────────┐
    │ PENDING │ ──────────────────▶│ READY       │
    │         │  threshold met     │ (threshold) │
    └─────────┘                    └─────────────┘
                                         │
                                         │ executeMessage()
                                         ▼
                                   ┌─────────────┐
                                   │ PRE_EXEC    │
                                   │ (modules)   │
                                   └─────────────┘
                                         │
                                         │ pre-checks pass
                                         ▼
                                   ┌─────────────┐
                                   │ EXECUTING   │
                                   │ (target)    │
                                   └─────────────┘
                                         │
                                         │ call returns
                                         ▼
                                   ┌─────────────┐
                                   │ POST_EXEC   │
                                   │ (modules)   │
                                   └─────────────┘
                                         │
                                         │ post-checks pass
                                         ▼
                                   ┌─────────────┐
                                   │ COMPLETED   │
                                   └─────────────┘

    Any stage can transition to REJECTED on failure
```

```solidity
enum MessageStage {
    Pending,      // Collecting signatures
    Ready,        // Threshold met, awaiting execution
    PreExecution, // Running pre-execution modules
    Executing,    // Calling target contract
    PostExecution,// Running post-execution modules
    Completed,    // Successfully executed
    Rejected      // Failed at some stage (terminal)
}

struct MessageState {
    MessageStage stage;
    string messageType;
    bytes payload;
    uint256 value;
    uint64 nonce;
    uint64 timestamp;
    bytes32 appId;
    uint256 signaturesCollected;
}
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
        state.stage = MessageStage.Rejected;
        emit MessageRejected(messageId, "Execution failed", returnData);
        revert ExecutionFailed(returnData);
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

Pre and post-execution modules provide extensible validation:

```solidity
interface IModule {
    /**
     * Check if a message passes this module's validation
     * @param messageId The message being validated
     * @param stage Whether this is pre or post execution
     * @return pass True if validation passes
     * @return reason Explanation if validation fails
     */
    function check(bytes32 messageId, bool isPreExecution)
        external view returns (bool pass, string memory reason);
}

// Module types
contract RateLimitModule is IModule {
    // Limit messages per time window
}

contract AmountThresholdModule is IModule {
    // Flag/delay large value transfers
}

contract AllowlistModule is IModule {
    // Restrict to known addresses
}

contract TimelockModule is IModule {
    // Delay high-value operations
}
```

### 6.7 Events

These are enshrined Bridge events (not module events). The bridge-wide signature threshold can be extended for specific sensitive transactions via pre-execution modules.

```solidity
// Signature events
event SignatureSubmitted(bytes32 indexed messageId, address indexed validator, uint256 count);
event ThresholdReached(bytes32 indexed messageId, uint256 signatures);

// Execution events
event MessageExecuted(bytes32 indexed messageId, string messageType, address target);
event MessageRejected(bytes32 indexed messageId, string reason, bytes data);

// Validator events
event ValidatorAdded(address indexed validator, bytes attestation);
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

CLAUDE: Rewrite this to differentiate between the validator that connects to
the application vs the validators that connect to the storage layers.

### 7.2 Single Validator Mode

**Use Case**: Non-financial applications, low-value operations, development/testing

```
Application ───▶ Single Validator ───▶ Bridge
                       │
                       └───▶ DA Layer (audit)
```

**Configuration**:

```solidity
signatureThreshold = 1;
validators = [validatorAddress];
```

**Characteristics**:

- Fastest path to execution (single signature)
- Single point of trust (the validator)
- Bridge still enforces all rules
- Suitable for: gaming state, social feeds, metadata updates, internal tools

**Trust assumption**: You trust the validator operator not to sign malicious messages.

### 7.3 Multi-Validator Mode

**Use Case**: Financial applications, high-value operations, production systems

```
                   ┌───▶ Validator 1 ───┐
                   │                    │
Application ───────┼───▶ Validator 2 ───┼───▶ Bridge (M-of-N)
                   │                    │
                   └───▶ Validator N ───┘
                              │
                              └───▶ DA Layer (audit)
```

**Configuration**:

```solidity
signatureThreshold = 2;  // 2-of-3
validators = [validator1, validator2, validator3];
```

**Characteristics**:

- Byzantine fault tolerant (tolerates (N-M) malicious validators)
- Distributed trust (no single point of failure)
- Higher latency (multiple signatures needed)
- Suitable for: token bridges, DeFi, high-value transfers

**Common configurations**:

- 2-of-3: Tolerates 1 malicious/offline validator
- 3-of-5: Tolerates 2 malicious/offline validators
- 5-of-9: High security for critical operations

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

CLAUDE: A lot of these levels should be included in validator metadata. Rewrite
this to focus on validator metadata best practices (i.e. more metadata from the
application gives the validators more information to work with)

### 7.6 Application Security Upgrades

Applications can progressively upgrade their security without protocol changes:

**Level 1: Basic**

- HTTP application, no special security
- Trust application operator for business logic

**Level 2: Logging**

- Comprehensive audit logs
- Validators can verify claims against logs

**Level 3: TEE**

- Application runs in Trusted Execution Environment
- TEE attestation proves application integrity

**Level 4: Checkpointing**

- Periodic database/state snapshots
- Validators can re-derive application state

**Level 5: Full Verification**

- Application code is open and deterministic
- Validators re-execute all logic

The protocol supports all levels; applications choose based on value at risk.

---

CLAUDE: Explain how TEEs help here. Especially why even though an application
running in a TEE is optional, it is strongly recommended for financial use
cases.

## 8. Security Analysis

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

**Scenario 4: Validator key theft**

```
Attacker: Steal validator private key
Mitigation:
1. Keys stored in TEE (hardware protected)
2. M-of-N threshold requires multiple keys
3. Key rotation procedures
4. Monitoring for anomalous signing patterns
```

### 8.5 Security Recommendations

CLAUDE: The application will also manage their own bridge. But we would
recommend a security council to avoid unilateral control, in addition to the
good steps you've outlined here.
**For Bridge Operators**:

1. Use multisig for admin operations
2. Implement timelocks for sensitive changes
3. Regular security audits of contracts
4. Monitoring and alerting for anomalies
5. Incident response procedures

CLAUDE: Extend with custom validation logic based on available metadata?
**For Validator Operators**:

1. Run validators in TEE
2. Implement key rotation
3. Monitor signing patterns
4. Rate limit per application
5. Geographic distribution

**For Application Developers**:

1. Use appropriate validator configuration for value at risk
2. Implement nonce management correctly
3. Add application-level rate limits
4. Log all message submissions
5. Monitor for failed validations

---

## 9. Data Availability Integration

### 9.1 Purpose

The DA layer serves audit and transparency purposes:

1. **Audit Trail**: Complete history of all validated messages
2. **Dispute Resolution**: Evidence for challenging fraudulent operations
3. **Compliance**: Regulatory transparency requirements
4. **Recovery**: Reconstruct message history if needed

The DA layer is for auditability, not consensus. Validators do not need to wait for DA confirmation before submitting to Bridge.

### 9.2 Publication Format

Messages published to DA include:

```typescript
interface DARecord {
  // The original message
  message: {
    id: bytes32;
    messageType: string;
    metadata: object;
    nonce: uint64;
    timestamp: uint64;
    appId: bytes32;
    value?: uint256;
  };

  // Validator attestations
  signatures: Array<{
    validator: address;
    signature: bytes;
    signedAt: uint64;
    teeAttestation?: string; // Optional TEE proof
  }>;

  // Publication metadata
  publication: {
    publishedBy: address; // Validator that published
    publishedAt: uint64;
    daLayer: string; // "celestia", "arweave", "ipfs"
    daReference: string; // Layer-specific reference
  };

  // Execution result (if known at publication time)
  execution?: {
    bridgeTxHash: bytes32;
    blockNumber: uint64;
    success: boolean;
  };
}
```

CLAUDE: We should always publish before execution, publishing after execution
doesn't make sense since validators read from it. We should recommend batching
for most use cases, and only recommend immediate publication if the use case is
very latency sensitive.

### 9.3 Publication Modes

**Immediate Publication** (Default)

- Validator publishes after signing, before Bridge submission
- Provides pre-execution audit trail
- Higher latency but maximum transparency

```
Application → Validator → [Sign] → [Publish to DA] → [Submit to Bridge]
```

**Batched Publication**

- Validators batch multiple messages for efficiency
- Publish every N seconds or M messages
- Lower DA costs, slight delay in audit availability

```
Application → Validator → [Sign] → [Buffer] → [Batch Publish to DA] → [Submit to Bridge]
```

**Post-Execution Publication**

- Publish after Bridge confirms execution
- Includes execution result in DA record
- Complete end-to-end audit in single record

```
Application → Validator → [Sign] → [Submit to Bridge] → [Wait for confirmation] → [Publish to DA]
```

CLAUDE: We should differentiate between storage and DA options here. Also add
Celestia and EigenDA for DA. Prioritize recommending storage layers over DA
layers for archival purposes.

### 9.4 DA Layer Options

| Layer        | Cost   | Latency | Durability        | Best For              |
| ------------ | ------ | ------- | ----------------- | --------------------- |
| **Celestia** | Low    | ~12s    | Network consensus | High-volume, low-cost |
| **Arweave**  | Medium | ~5min   | Permanent         | Compliance, archival  |
| **IPFS**     | Free\* | Instant | Best-effort       | Development, testing  |
| **GCS/S3**   | Low    | Instant | Centralized       | Internal audit only   |

\*IPFS requires pinning service for durability

### 9.5 Validator DA Configuration

```yaml
# Validator configuration
da:
  # Primary DA layer
  primary: celestia

  # Fallback if primary fails
  fallback: arweave

  # Publication mode
  mode: immediate # immediate | batched | post_execution

  # Batching settings (if mode: batched)
  batch:
    max_messages: 100
    max_delay_seconds: 30

  # Layer-specific config
  celestia:
    rpc_url: "https://celestia-rpc.example.com"
    namespace: "synd_bridge_01"
    auth_token: "${CELESTIA_AUTH_TOKEN}"

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

## 11. Migration from SQLite Replication

### 11.1 Coexistence Period

Both systems can run in parallel:

- Legacy: synddb-client + synddb-sequencer + synddb-validator (SQL replay mode)
- New: synddb-validator (message-passing mode) only

Bridge accepts messages from both sources during migration.

CLAUDE: NO MIGRATION IS NEEDED! WE DO NOT NEED TO MAINTAIN ANY BACKWARD
COMPATIBILITY WITH SYNDDB. SYNDDB IS NOT DEPLOYED TO PRODUCTION SO BACKWARD
COMPATIBILITY IS NOT A CONCERN.

### 11.2 Migration Steps

1. **Analyze Existing Messages**
   - Review withdrawal/message tables in current system
   - Map to message types

2. **Define Schemas**
   - Create JSON Schema for each message type
   - Register on Bridge

3. **Update Application**
   - Replace SQLite writes with HTTP POST
   - Remove synddb-client dependency
   - Implement nonce management

4. **Parallel Testing**
   - Run both systems simultaneously
   - Compare results
   - Verify consistency

5. **Switch Over**
   - Disable legacy path
   - Monitor for issues
   - Decommission old components

### 11.3 Component Changes

| Component        | Change                                         |
| ---------------- | ---------------------------------------------- |
| synddb-client    | Deprecated (remove from application)           |
| synddb-sequencer | Removed (functionality absorbed by validators) |
| synddb-validator | New mode added (`--mode message-passing`)      |
| synddb-shared    | New message types, remove changeset types      |
| Bridge.sol       | Extended with message type registry            |

---

## 12. Appendix

### A. Example Integration

**Minimal Application Integration**:

```python
import requests
import hashlib
import time

VALIDATOR_URL = "https://validator.example.com"
APP_ID = "0x" + "00" * 31 + "01"

nonce = 0

def submit_message(message_type: str, metadata: dict, value: int = 0):
    global nonce
    nonce += 1

    message = {
        "messageType": message_type,
        "metadata": metadata,
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
message_id = submit_message(
    "mint(address,uint256)",
    {
        "recipient": "0x742d35Cc6634C0532925a3b844Bc454e4438f44e",
        "amount": "1000000000000000000",
        "reason": "user_deposit"
    }
)
print(f"Submitted: {message_id}")
```

### B. JSON Schema Examples

**ERC20 Transfer**:

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "transfer(address,address,uint256)",
  "type": "object",
  "required": ["from", "to", "amount"],
  "properties": {
    "from": {
      "type": "string",
      "pattern": "^0x[a-fA-F0-9]{40}$"
    },
    "to": {
      "type": "string",
      "pattern": "^0x[a-fA-F0-9]{40}$"
    },
    "amount": {
      "type": "string",
      "pattern": "^[0-9]+$"
    }
  }
}
```

**NFT Batch Mint**:

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "batchMint(address[],uint256[],string[])",
  "type": "object",
  "required": ["recipients", "tokenIds", "tokenURIs"],
  "properties": {
    "recipients": {
      "type": "array",
      "items": {
        "type": "string",
        "pattern": "^0x[a-fA-F0-9]{40}$"
      },
      "minItems": 1,
      "maxItems": 100
    },
    "tokenIds": {
      "type": "array",
      "items": {
        "type": "string",
        "pattern": "^[0-9]+$"
      }
    },
    "tokenURIs": {
      "type": "array",
      "items": {
        "type": "string",
        "format": "uri"
      }
    }
  }
}
```

### C. Glossary

| Term             | Definition                                                                       |
| ---------------- | -------------------------------------------------------------------------------- |
| **Message Type** | ABI function signature identifying the operation (e.g., `mint(address,uint256)`) |
| **Metadata**     | JSON payload containing operation parameters                                     |
| **Schema**       | JSON Schema defining required/optional metadata fields                           |
| **Validator**    | Service that validates messages and signs them                                   |
| **Bridge**       | Smart contract that aggregates signatures and executes messages                  |
| **Module**       | Pluggable validation component for pre/post execution checks                     |
| **DA Layer**     | Data Availability layer for audit trail storage                                  |
| **Threshold**    | Minimum number of validator signatures required                                  |
| **Nonce**        | Monotonically increasing counter for replay protection                           |
| **TEE**          | Trusted Execution Environment (e.g., Intel SGX, AMD SEV)                         |
