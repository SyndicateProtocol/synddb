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

- Constructs messages with correct type and metadata
- Sends messages to validator endpoint(s)
- No blockchain interaction required
- Responsible for its own security based on value at risk

**Validator**

- Validates message types against Bridge registry
- Validates metadata against JSON Schemas
- Applies custom validation rules
- Signs valid messages with TEE-protected keys
- Publishes to DA layer for audit trail
- Submits signatures to Bridge

**Bridge (Smart Contract)**

- Maintains registry of allowed message types
- Stores schema hashes for metadata validation
- Aggregates validator signatures
- Enforces signature threshold
- Executes messages via modular pre/post hooks
- Controls funds and enforces security invariants

**DA Layer**

- Stores all validated messages for audit
- Enables historical replay
- Provides evidence for dispute resolution
- Supports compliance requirements

---

## 3. Message Format

### 3.1 Message Structure

Messages consist of a type signature and metadata payload:

```typescript
interface Message {
  // Unique identifier (keccak256 hash of content, prevents replay)
  id: bytes32;

  // Human-readable ABI function signature
  // Examples: "mint(address,uint256)", "transfer(address,address,uint256)"
  messageType: string;

  // JSON metadata payload - validated against schema
  metadata: object;

  // Monotonically increasing nonce per application (prevents replay/reorder)
  nonce: uint64;

  // Unix timestamp in seconds
  timestamp: uint64;

  // Application identifier (for multi-tenant validators)
  appId: bytes32;

  // Optional: Native token amount to send with execution
  value?: uint256;
}
```

### 3.2 Message Type Format

Message types use Solidity function signature format:

```
messageType = "functionName(type1,type2,...)"
```

Examples:

- `mint(address,uint256)` - Mint tokens to address
- `transfer(address,address,uint256)` - Transfer between addresses
- `safeMint(address,uint256,string)` - Mint NFT with URI
- `batchTransfer(address[],uint256[])` - Batch transfer
- `execute(address,bytes)` - Generic execution

The 4-byte function selector is derived as: `bytes4(keccak256(messageType))`

### 3.3 Metadata Payload

Metadata is a JSON object containing the parameters for the message type. The fields must match the registered JSON Schema for that message type.

**Example: ERC20 Mint**

```json
{
  "id": "0x1234567890abcdef...",
  "messageType": "mint(address,uint256)",
  "metadata": {
    "recipient": "0x742d35Cc6634C0532925a3b844Bc454e4438f44e",
    "amount": "1000000000000000000",
    "reason": "user_deposit",
    "sourceChain": "ethereum",
    "sourceTxHash": "0xabcdef..."
  },
  "nonce": 42,
  "timestamp": 1735084800,
  "appId": "0x0000000000000000000000000000000000000001"
}
```

**Example: NFT Mint**

```json
{
  "id": "0xfedcba0987654321...",
  "messageType": "safeMint(address,uint256,string)",
  "metadata": {
    "to": "0x742d35Cc6634C0532925a3b844Bc454e4438f44e",
    "tokenId": "12345",
    "tokenURI": "ipfs://QmXyz..."
  },
  "nonce": 43,
  "timestamp": 1735084801,
  "appId": "0x0000000000000000000000000000000000000001"
}
```

### 3.4 Message ID Generation

The message ID is computed as:

```solidity
bytes32 messageId = keccak256(abi.encode(
    messageType,
    keccak256(abi.encode(metadata)),
    nonce,
    timestamp,
    appId
));
```

This ensures:

- Unique ID for each distinct message
- Replay protection via nonce
- Tamper detection (any change invalidates signatures)

### 3.5 Payload Encoding for Execution

When the Bridge executes a message, metadata is ABI-encoded:

```solidity
// For messageType = "mint(address,uint256)"
// metadata = { "recipient": "0x123...", "amount": "1000" }
bytes memory calldata = abi.encodeWithSelector(
    bytes4(keccak256("mint(address,uint256)")),
    metadata.recipient,
    metadata.amount
);

// Execute on target
target.call{value: msg.value}(calldata);
```

---

## 4. Validation Protocol

### 4.1 Validator Processing Flow

When a validator receives a message, it processes through these stages:

```
RECEIVE → VALIDATE → SIGN → PUBLISH → SUBMIT
```

**Stage 1: Receive**

```
POST /messages
Content-Type: application/json

{
  "messageType": "mint(address,uint256)",
  "metadata": { ... },
  "nonce": 42,
  "timestamp": 1735084800,
  "appId": "0x..."
}
```

**Stage 2: Validate**

The validator performs multiple validation checks in sequence:

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
│  2.5 SCHEMA VALIDATION                                           │
│      ├─ Fetch schema (from cache, chain, or IPFS/Arweave)        │
│      ├─ Validate metadata against JSON Schema                    │
│      └─ Check: all required fields present with correct types    │
│                                                                  │
│  2.6 CUSTOM RULES (optional, validator-specific)                 │
│      ├─ Rate limiting (messages per second/minute)               │
│      ├─ Amount thresholds (flag large transfers)                 │
│      ├─ Business logic (game rules, allowlists)                  │
│      └─ External verification (check source chain, etc.)         │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

**Stage 3: Sign**

If all validations pass:

1. Compute the signing payload (EIP-712 typed data or raw hash)
2. Sign with validator's private key (protected by TEE)
3. Attach TEE attestation token if available

**Stage 4: Publish (DA Layer)**

Publish signed message to DA layer for audit:

1. Serialize message + signature + attestation
2. Submit to configured DA backend (Celestia, Arweave, etc.)
3. Store DA reference for future queries

**Stage 5: Submit (Bridge)**

Submit signature to Bridge for on-chain aggregation:

1. Call `Bridge.signMessage(messageId, signature)`
2. Or return signature to relayer for batched submission

### 4.2 Validation Levels

Validation is hierarchical, with different levels enforced by different components:

| Level        | Enforced By           | Examples                                               |
| ------------ | --------------------- | ------------------------------------------------------ |
| **Protocol** | All Validators        | Replay protection, nonce ordering, timestamp freshness |
| **Bridge**   | Smart Contract        | Message type registration, signature threshold         |
| **Schema**   | Validators + Bridge   | Required fields, field types, value constraints        |
| **Custom**   | Individual Validators | Rate limits, business rules, external checks           |

### 4.3 Validator HTTP API

```yaml
# Submit a message for validation and signing
POST /messages
  Request:
    messageType: string      # Required: ABI signature
    metadata: object         # Required: JSON payload
    nonce: uint64            # Required: Application nonce
    timestamp: uint64        # Required: Unix timestamp
    appId: bytes32           # Required: Application ID
    value?: uint256          # Optional: Native token amount
  Response:
    status: "accepted" | "rejected"
    messageId?: bytes32      # If accepted
    signature?: bytes        # Validator signature
    daReference?: string     # DA layer reference
    error?: string           # If rejected

# Check status of a submitted message
GET /messages/{messageId}
  Response:
    id: bytes32
    status: "pending" | "signed" | "published" | "submitted" | "executed"
    signatures: address[]    # Validators who have signed
    daReference?: string
    bridgeTxHash?: bytes32   # If submitted to bridge

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
- `APP_NOT_AUTHORIZED` - Application ID not registered
- `MESSAGE_TYPE_NOT_REGISTERED` - Unknown message type
- `MESSAGE_TYPE_DISABLED` - Message type currently disabled
- `SCHEMA_VALIDATION_FAILED` - Metadata doesn't match schema
- `RATE_LIMIT_EXCEEDED` - Too many messages from this app
- `CUSTOM_RULE_FAILED` - Custom validation rule failed

---

## 5. Schema Registration

### 5.1 Overview

The Bridge maintains a registry of allowed message types. Each message type has:

- A target contract to call
- A JSON Schema defining required/optional metadata fields
- An enabled/disabled state

New message types are added via Bridge admin functions. Validators fetch and cache schemas to validate incoming messages.

### 5.2 Message Type Registry

The Bridge stores message type configurations:

```solidity
struct MessageTypeConfig {
    // Computed from messageType string
    bytes4 selector;

    // Contract to call when executing this message type
    address target;

    // Hash of the JSON Schema (keccak256)
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

Schemas use JSON Schema (draft 2020-12) to define metadata requirements:

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "mint(address,uint256)",
  "title": "ERC20 Mint",
  "description": "Mint tokens to a recipient address",
  "type": "object",
  "required": ["recipient", "amount"],
  "properties": {
    "recipient": {
      "type": "string",
      "pattern": "^0x[a-fA-F0-9]{40}$",
      "description": "Ethereum address to receive tokens"
    },
    "amount": {
      "type": "string",
      "pattern": "^[0-9]+$",
      "description": "Amount to mint (wei, as string for large numbers)"
    },
    "reason": {
      "type": "string",
      "enum": ["user_deposit", "reward", "airdrop", "migration"],
      "description": "Reason for minting (optional, for audit)"
    },
    "sourceChain": {
      "type": "string",
      "description": "Source chain for cross-chain mints"
    },
    "sourceTxHash": {
      "type": "string",
      "pattern": "^0x[a-fA-F0-9]{64}$",
      "description": "Source transaction hash for verification"
    }
  },
  "additionalProperties": true
}
```

Key points:

- `required` specifies mandatory fields
- `additionalProperties: true` allows extra metadata without schema changes
- Use `pattern` for format validation (addresses, hashes)
- Use `enum` for constrained values

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
     * @param payload ABI-encoded metadata (matches function parameters)
     * @param nonce Application nonce
     * @param timestamp Message timestamp
     * @param appId Application identifier
     * @param signatures Array of validator signatures
     */
    function submitMessage(
        bytes32 messageId,
        string calldata messageType,
        bytes calldata payload,
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

    /**
     * Add a validator to the set
     */
    function addValidator(address validator) external;

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

```solidity
// Signature events
event SignatureSubmitted(bytes32 indexed messageId, address indexed validator, uint256 count);
event ThresholdReached(bytes32 indexed messageId, uint256 signatures);

// Execution events
event MessageExecuted(bytes32 indexed messageId, string messageType, address target);
event MessageRejected(bytes32 indexed messageId, string reason, bytes data);

// Admin events
event ValidatorAdded(address indexed validator);
event ValidatorRemoved(address indexed validator);
event ThresholdUpdated(uint256 oldThreshold, uint256 newThreshold);
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

// Consider timelock for sensitive operations
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

**For Bridge Operators**:

1. Use multisig for admin operations
2. Implement timelocks for sensitive changes
3. Regular security audits of contracts
4. Monitoring and alerting for anomalies
5. Incident response procedures

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
