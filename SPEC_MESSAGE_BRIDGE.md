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

| Aspect | SQLite Replication (Old) | Message Passing (New) |
|--------|--------------------------|----------------------|
| Client Integration | Required SDK, SQLite hooks | HTTP POST |
| Data Format | SQLite changesets (binary) | JSON + ABI signature |
| Sequencer | Required (separate TEE) | Eliminated |
| Message Origin | Database writes | Direct API calls |
| Validation | SQL replay + invariants | Schema + custom rules |
| Developer Effort | High (library integration) | Low (HTTP calls) |

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
