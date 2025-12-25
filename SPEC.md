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

**Relayer**

Relayers are entities that call `executeMessage()` on the Bridge after signature threshold is met. This is an **untrusted, permissionless role** - anyone can relay.

| Aspect | Description |
|--------|-------------|
| **Role** | Delivers validated messages to the Bridge for execution |
| **Trust** | Untrusted - cannot forge signatures or bypass validation |
| **Incentive** | Out-of-band (application pays relayer, or validator self-relays) |
| **Who can relay** | Anyone - permissionless |

Common relayer patterns:
- **Validator as Relayer**: Primary or Witness Validator calls `executeMessage()` after signing
- **Dedicated Relayer Service**: Third-party service monitors for threshold-met messages
- **Application Self-Relay**: Application backend monitors and relays its own messages

*Note: This aligns with Hyperlane's "Relayer" and LayerZero's "Executor" terminology.*

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
     "domain": "0x...",
     "publicKey": "-----BEGIN CERTIFICATE-----...",
     "allowedMessageTypes": ["mint(address,uint256)", ...],
     "rateLimit": { "maxPerSecond": 100, "maxPerDay": 10000 }
   }
3. Validator stores registration, assigns domain
4. All subsequent requests use mTLS with client cert
5. Validator verifies: cert → domain → authorized message types
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
   Bridge.registerApplication(domain, primaryValidator, config)
4. Application receives domain and can begin submitting messages
```

**Recommended: Deterministic domain**

We recommend deriving `domain` deterministically to avoid coordination:

```solidity
// Option A: From application's on-chain address
bytes32 domain = keccak256(abi.encode(chainId, applicationAddress));

// Option B: From DNS name (for off-chain apps)
bytes32 domain = keccak256(abi.encode("app.example.com"));

// Option C: From Primary Validator + nonce (for multi-tenant validators)
bytes32 domain = keccak256(abi.encode(primaryValidator, registrationNonce));
```

**Bridge Registration Function**:

```solidity
struct ApplicationConfig {
    address primaryValidator;      // Who can initialize messages
    uint64 expirationSeconds;      // Message expiration (default: 86400 = 24h)
    bool requireWitnessSignatures; // Require multi-validator mode
}

function registerApplication(
    bytes32 domain,
    ApplicationConfig calldata config
) external;

function getApplicationConfig(bytes32 domain)
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
   - `Rejected` (Primary rejected via `rejectProposal()`)
3. All rejections are explicit and on-chain (nonce consumed)
4. Once a nonce is used (initialized OR rejected), it cannot be reused

**No Gap Tolerance**:

```
Valid:   nonce 1 → nonce 2 → nonce 3
Invalid: nonce 1 → nonce 3 (gap at 2)
```

If a message fails or expires, the application must continue with the next nonce. There is no retry-with-same-nonce mechanism.

**Primary Validator Nonce Tracking**:

The Primary Validator tracks `last_nonce` per domain. It validates that each incoming nonce equals `last_nonce + 1`, then consumes the nonce on both `initialize` and `reject` operations.

> See `PLAN_VALIDATORS.md` Section 2.2 for implementation.

### 2.7 Message Expiration

Messages expire if they don't reach signature threshold within the expiration window.

**Default**: 24 hours from message timestamp

**Expiration Check**:

Bridge provides `isExpired(messageId)` and `expireMessage(messageId)` functions. Expiration is computed as `message.timestamp + config.expirationSeconds`. Anyone can call `expireMessage()` for pending messages past their deadline.

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

  // Domain identifier - each application registers with validators
  // and receives a unique domain ID for message attribution and rate limiting
  // (Similar to Hyperlane's "domain" concept, but identifies application, not chain)
  domain: bytes32;

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
  "domain": "0x0000000000000000000000000000000000000001"
}
```

Note: The calldata contains the ABI-encoded `mint(0x742d..., 1000000000000000000)`. The metadata provides context validators use to decide whether to sign.

### 3.6 Invariant Specification

Invariants are constraints that must hold true for a message to be valid. Validators enforce these constraints before signing, and Bridge Modules can enforce additional constraints during execution.

#### 3.6.1 Invariant Categories

Invariants fall into three categories based on their data source:

| Category | Verified By | Data Source | Examples |
|----------|-------------|-------------|----------|
| **On-Chain State** | Validators (pre-sign) + Modules (post-exec) | RPC calls to contracts | Total supply caps, balance checks, allowance limits |
| **Off-Chain Oracle** | Validators (pre-sign) | Oracle feeds, price APIs | Price deviation thresholds, exchange rate bounds |
| **Application Logic** | Validators (pre-sign) | Metadata fields, optional app API | Game rules, rate limits, business constraints |

Each category has different trust properties and verification patterns.

#### 3.6.2 On-Chain State Invariants

On-chain invariants verify that blockchain state satisfies required conditions. Validators query contract state via RPC and compare against limits specified in metadata or schema.

**Common Patterns**:

| Pattern | What Validators Check | Data Source |
|---------|----------------------|-------------|
| Supply Cap | `currentSupply + mintAmount <= maxSupply` | `totalSupply()` on target contract |
| Balance Check | `senderBalance >= transferAmount` | `balanceOf(sender)` on token contract |
| Allowance Check | `allowance >= spendAmount` | `allowance(owner, spender)` on token contract |
| Ownership | `ownerOf(tokenId) == expectedOwner` | `ownerOf(tokenId)` on NFT contract |

**Pre-Execution vs Post-Execution**:

- **Pre-execution checks** (by validators): Verify state *before* the message is signed. Use when current state determines validity.
- **Post-execution checks** (by Bridge Modules): Verify state *after* execution completes. Use when the invariant depends on the result of the operation (e.g., "total supply must still be <= max after this mint").

**Example: Total Supply Cap**

For a `mint(address,uint256)` message, validators enforce that minting won't exceed the maximum supply:

```json
{
  "messageType": "mint(address,uint256)",
  "calldata": "0x40c10f19...",
  "metadata": {
    "reason": "user_deposit",
    "maxSupply": "10000000000000000000000000",
    "sourceChain": "ethereum",
    "sourceTxHash": "0xabc..."
  }
}
```

**Validator logic**: Extract mint amount from calldata, query `totalSupply()` via RPC, verify `currentSupply + amount <= maxSupply`.

**Witness behavior**: Witnesses independently query state via their own RPC. Minor timing differences are expected.

**Bridge Module**: For critical supply caps, a post-execution `SupplyCapModule` provides on-chain enforcement.

> See `PLAN_VALIDATORS.md` Section 3.1 and `PLAN_CONTRACTS.md` Section 3.4 for implementation.

#### 3.6.3 Off-Chain Oracle Invariants

Oracle invariants verify that off-chain data (prices, exchange rates, external state) satisfies required conditions. Validators fetch data from oracles or APIs and compare against thresholds.

**Common Patterns**:

| Pattern | What Validators Check | Data Source |
|---------|----------------------|-------------|
| Price Deviation | `|appPrice - oraclePrice| / oraclePrice <= threshold` | Chainlink, Pyth, Redstone |
| Rate Bounds | `minRate <= exchangeRate <= maxRate` | Price API, DEX oracle |
| Freshness | `oracleTimestamp >= now - maxAge` | Oracle heartbeat |

**Freshness Requirements**:

Oracle data can become stale. Validators should:
1. Check the oracle's last update timestamp
2. Reject if data is older than a configured maximum age (e.g., 5 minutes for volatile assets)
3. Use time-weighted averages (TWAP) for manipulation resistance when appropriate

**Example: Price Deviation Threshold**

Metadata includes `exchangeRate`, `maxDeviation`, token identifiers. Validator fetches oracle price, checks freshness, calculates deviation percentage, rejects if exceeds threshold.

**Supported Oracle Providers**: Chainlink (on-chain), Pyth (cross-chain), Redstone, Custom API

> See `PLAN_VALIDATORS.md` Section 3.2 for implementation.

#### 3.6.4 Application Logic Invariants

Application logic invariants enforce business rules specific to the application. These cannot be independently verified from public data sources - they depend on application state.

**Common Patterns**:

| Pattern | What Validators Check | Data Source |
|---------|----------------------|-------------|
| Rate Limiting | User hasn't exceeded action frequency | App database/API |
| Game Rules | Game state allows this action | Game server API |
| Access Control | User has permission for this action | App permission system |
| Uniqueness | Action hasn't already been performed | App state |

**Trust Model**:

Application logic invariants have a different trust model than on-chain or oracle invariants:

- **Primary Validator**: Validates based on metadata fields provided by the application. Trusts the application to report accurate state.
- **Witness Validator**: Cannot independently verify application state. Options:
  1. Trust Primary's validation (default)
  2. Query application's Verification API (if provided)
  3. Require application to run in TEE with attestation

**Example: Game Invariants**

Metadata includes `gameWinnerCount`, `playerLastPlayTimestamp`, `minPlayInterval`. Validator checks:
- `gameWinnerCount == 0` (only one winner per game)
- `currentTimestamp - playerLastPlayTimestamp >= minPlayInterval` (play cooldown)

If `verificationApiUrl` is provided, Witnesses can independently verify via the application's API.

> See `PLAN_VALIDATORS.md` Section 3.3-3.4 for implementation.

#### 3.6.5 Verification API Pattern (Optional)

Applications can implement a Verification API (`POST /verify`) to allow Witness Validators to independently verify application logic invariants. Request includes `messageId` and `metadata`; response returns `valid: boolean` with per-invariant results.

> See `PLAN_VALIDATORS.md` Section 3.4 for API specification.

**When to Implement**:

| Scenario | Recommendation |
|----------|----------------|
| Internal tools, low value | Skip - trust Primary |
| Gaming, moderate value | Implement - adds Witness verification |
| Financial, high value | Implement + run app in TEE |
| Multi-tenant bridge | Required - Witnesses need independent verification |

**Trust Hierarchy**:

| Verification Method | Trust Level | Use Case |
|---------------------|-------------|----------|
| Trust Primary only | Low | Internal apps, testing |
| Verification API | Medium | Production apps |
| Verification API + App in TEE | High | Financial apps |
| Open source + deterministic replay | Maximum | Critical infrastructure |

#### 3.6.6 Schema-Level vs Runtime Invariants

Invariants can be enforced at two levels:

**Schema-Level** (in JSON Schema):
- Field presence and types (`required`, `type`)
- Value constraints (`minimum`, `maximum`, `pattern`)
- Enum restrictions (`enum`)
- Field relationships (`if/then/else` in JSON Schema)

**Runtime** (in validator logic):
- Cross-field calculations (`amount <= balance`)
- External data queries (`currentSupply + amount <= maxSupply`)
- Temporal constraints (`now - lastAction >= minInterval`)
- Business logic (`gameWinnerCount == 0`)

**Example: Combined Enforcement**

Schema ensures required fields and types:

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "required": ["maxSupply", "reason"],
  "properties": {
    "maxSupply": {
      "type": "string",
      "pattern": "^[0-9]+$",
      "description": "Maximum token supply (required for supply cap invariant)"
    },
    "reason": {
      "type": "string",
      "enum": ["user_deposit", "reward", "migration"],
      "description": "Reason for minting"
    }
  }
}
```

Validator logic enforces runtime invariants: first run schema validation, then check supply cap via RPC, then check rate limits. All checks must pass.

### 3.7 Message ID Generation

Message IDs follow standard Ethereum hashing patterns:

```solidity
bytes32 messageId = keccak256(abi.encode(
    messageType,
    keccak256(calldata),
    keccak256(abi.encode(metadata)),  // Metadata included in ID
    nonce,
    timestamp,
    domain
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

Canonicalize the metadata object using RFC 8785 rules, then compute `keccak256(canonical_bytes)`. Use `separators=(',', ':')` and `sort_keys=True` when serializing JSON.

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
  "domain": "0x..."
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
│      └─ Check: nonce > lastNonce[domain]                          │
│                                                                  │
│  2.2 FRESHNESS CHECK                                             │
│      └─ Check: |timestamp - now| < MAX_CLOCK_DRIFT               │
│                                                                  │
│  2.3 APPLICATION AUTHORIZATION                                   │
│      └─ Check: domain is registered and active                    │
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
│  2.7 INVARIANT CHECKS (see Section 3.6 for details)              │
│      ├─ On-chain state invariants:                               │
│      │   └─ Query contract state via RPC (totalSupply, balances) │
│      │   └─ Compare against limits in metadata/schema            │
│      ├─ Off-chain oracle invariants:                             │
│      │   └─ Fetch price/rate from oracle (Chainlink, etc.)       │
│      │   └─ Check freshness, verify deviation within threshold   │
│      ├─ Application logic invariants:                            │
│      │   └─ Validate metadata fields satisfy business rules      │
│      │   └─ (e.g., gameWinnerCount == 0, play cooldown met)      │
│      └─ On any invariant failure: REJECT message                 │
│                                                                  │
│  2.8 CUSTOM RULES (validator-specific)                           │
│      ├─ Rate limiting (messages per second/minute)               │
│      ├─ Amount thresholds (flag large transfers)                 │
│      └─ External verification (check source chain, etc.)         │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

**Primary Rejection Path**:

If any validation check fails, the Primary Validator **explicitly rejects on-chain**. All rejections must be logged on-chain for auditability.

```
Validation Failed
      │
      ▼
┌─────────────────────────────────────────────────────────────────┐
│                     PRIMARY REJECTION                            │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  1. Compute messageId from proposed message content              │
│                                                                  │
│  2. Call Bridge.rejectProposal() with:                           │
│     - messageId                                                  │
│     - messageType                                                │
│     - domain                                                      │
│     - nonce                                                      │
│     - reasonHash (hash of rejection reason)                      │
│     - reasonRef (storage reference to full reason)               │
│                                                                  │
│  3. Bridge logs rejection event on-chain                         │
│                                                                  │
│  4. Return error response to application (HTTP 400/422)          │
│                                                                  │
│  5. Nonce is consumed (prevents replay of rejected message)      │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

**On-Chain Rejection Function**:

`rejectProposal(messageId, messageType, domain, nonce, reasonHash, reasonRef)` - Only callable by the Primary Validator for the domain. Marks the proposal as rejected, consumes the nonce, and emits `ProposalRejected` event.

> See `PLAN_CONTRACTS.md` Section 6.2 for implementation.

**Key Points**:
- All rejections are explicit and on-chain (both Primary and Witness)
- Primary uses `rejectProposal()` for messages never initialized
- Witness uses `rejectMessage()` for already-initialized messages
- Nonce IS consumed on rejection (prevents replay)
- Application must use next nonce for retry

**Nonce behavior on Primary rejection**:
- Nonce IS consumed (on-chain rejection marks nonce as used)
- Application must increment nonce for retry
- This provides replay protection for rejected messages

**Stage 3: Sign**

Use **EIP-712 typed data signing**. The digest includes: messageId, messageType, calldata hash, metadata hash, nonce, timestamp, domain.

Validators are registered during TEE bootstrapping (Section 6.9). No per-message attestation required.

> See `PLAN_VALIDATORS.md` Section 7 for EIP-712 implementation.

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

**Stage 3b: Invariant Re-Verification**

Witness Validators re-verify invariants based on category (see Section 3.6):

```
┌─────────────────────────────────────────────────────────────────┐
│                  INVARIANT RE-VERIFICATION                       │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  On-chain state invariants:                                      │
│    ├─ Query contract state via own RPC connection                │
│    ├─ Re-evaluate all conditions (supply caps, balances)         │
│    └─ Minor timing differences expected; reject if violated      │
│                                                                  │
│  Off-chain oracle invariants:                                    │
│    ├─ Query same or different oracle source                      │
│    ├─ Check freshness independently                              │
│    └─ Accept minor deviations due to timing; reject if large     │
│                                                                  │
│  Application logic invariants:                                   │
│    ├─ Option A: Trust Primary's validation (default)             │
│    ├─ Option B: Query app's Verification API (if provided)       │
│    │   └─ POST to verificationApiUrl with message details        │
│    └─ Option C: Require app TEE attestation for high-value       │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

**Verification API**: If metadata includes `verificationApiUrl`, Witnesses call the app's API to independently verify invariants.

> See `PLAN_VALIDATORS.md` Section 3.4 for implementation.

**Trust Implications**:

| Invariant Type | Witness Can Re-verify? | Trust Model |
|----------------|----------------------|-------------|
| On-chain state | Yes (own RPC) | Independent verification |
| Oracle data | Yes (query oracle) | Independent verification |
| App logic (no API) | No | Trust Primary |
| App logic (with API) | Yes | Independent verification |
| App logic (TEE app) | Attestation | Cryptographic proof |

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
| **Protocol** | All Validators | Replay protection, nonce, timestamp, domain |
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
    domain: bytes32           # Application ID
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
    lastProcessedNonce: map[domain => nonce]

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
- `INVARIANT_VIOLATED` - Invariant condition failed (on-chain, oracle, or app logic)
- `INVARIANT_DATA_STALE` - Data source value exceeds freshness requirements
- `INVARIANT_DATA_UNAVAILABLE` - Could not fetch required data from source (RPC, oracle, or API)
- `INVARIANT_VERIFICATION_FAILED` - Verification API returned invalid or error response
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

Each registered message type has:
- `selector` - Function selector computed from message type string
- `target` - Contract to call when executing
- `schemaHash` - keccak256 of metadata JSON Schema
- `schemaUri` - Where to fetch full schema (IPFS, Arweave)
- `enabled` - Whether currently active

> See `PLAN_CONTRACTS.md` for `MessageTypeConfig` struct definition.

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

| Function | Description |
|----------|-------------|
| `registerMessageType(messageType, target, schemaHash, schemaUri)` | Register new message type |
| `updateSchema(messageType, newSchemaHash, newSchemaUri)` | Update schema for existing type |
| `setEnabled(messageType, enabled)` | Enable/disable message type |
| `setTarget(messageType, newTarget)` | Update target contract |
| `isRegistered(messageType)` | Query if type exists |
| `isEnabled(messageType)` | Query if type is active |
| `getConfig(messageType)` | Get full MessageTypeConfig |

> See `PLAN_CONTRACTS.md` for full interface.

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

**Message Initialization** (Primary Validator only):

| Function | Description |
|----------|-------------|
| `initializeMessage(messageId, messageType, calldata_, metadataHash, storageRef, nonce, timestamp, domain)` | Initialize a new message on Bridge |
| `initializeAndSign(...)` | Initialize and sign in one transaction |

**Signature Submission**:

| Function | Description |
|----------|-------------|
| `signMessage(messageId, signature)` | Submit validator's EIP-712 signature |
| `rejectMessage(messageId, reasonHash, reasonRef)` | Witness rejects initialized message |
| `rejectProposal(messageId, messageType, domain, nonce, reasonHash, reasonRef)` | Primary rejects before initialization |

**Execution**:

| Function | Description |
|----------|-------------|
| `executeMessage(messageId)` | Execute after threshold met (permissionless) |

**Validator Management** (requires TEE bootstrapping, see Section 6.9):

| Function | Description |
|----------|-------------|
| `setPrimaryValidator(domain, validator, attestation)` | Register Primary for application |
| `addWitnessValidator(validator, attestation)` | Add Witness to signing set |
| `removeValidator(validator)` | Remove validator |
| `setSignatureThreshold(threshold)` | Set M-of-N threshold |

**Queries**: `getMessageState`, `getSignatureCount`, `getRejectionCount`, `hasValidatorSigned`, `hasValidatorRejected`, `isMessageExecuted`, `getPrimaryValidator`, `getWitnessValidators`, `getSignatureThreshold`

> See `PLAN_CONTRACTS.md` for full `IMessageBridge` interface.

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

**Message Stages**: `NotInitialized` → `Pending` → `Ready` → `PreExecution` → `Executing` → `PostExecution` → `Completed`

Terminal states: `Completed`, `Failed`, `Expired` (all consume nonce)

> See `PLAN_CONTRACTS.md` Section 2.1 for struct definitions.

### 6.4 Signature Verification

Signatures use **EIP-712 typed data** for structured, verifiable signing. The digest includes messageId, messageType, calldata hash, metadata hash, nonce, timestamp, and domain.

> See `PLAN_CONTRACTS.md` Section 2.2 for implementation details.

### 6.5 Execution Logic

Execution proceeds through stages: Ready → PreExecution → Executing → PostExecution → Completed (or Failed).

1. Verify message is Ready (threshold met)
2. Run pre-execution modules
3. Call target contract with calldata and value
4. If failed, mark as Failed (terminal) and emit event
5. Run post-execution modules
6. Mark as Completed and emit event

> See `PLAN_CONTRACTS.md` Section 2.3 for implementation details.

### 6.6 Module System

Modules provide extensible pre/post-execution validation.

**Module Scope**:

| Scope | Applies To | Use Case |
|-------|------------|----------|
| **Global** | All message types | Rate limiting, monitoring |
| **Per-Message-Type** | Specific message types | Amount thresholds, timelocks |

**Module Interface**:

```solidity
interface IModule {
    function check(bytes32 messageId, bool isPreExecution)
        external view returns (bool pass, string memory reason);
}
```

**Common Module Types**:

| Module | Timing | Purpose |
|--------|--------|---------|
| RateLimitModule | Pre | Limit messages per time window |
| AmountThresholdModule | Pre | Flag/delay large transfers |
| AllowlistModule | Pre | Restrict to known addresses |
| TimelockModule | Pre | Delay sensitive operations |
| SupplyCapModule | Post | Verify supply invariants |

> See `PLAN_CONTRACTS.md` Section 3 for implementation details.

### 6.7 Events

Bridge emits events for all state changes:

| Category | Events |
|----------|--------|
| Message Lifecycle | `MessageInitialized`, `SignatureSubmitted`, `MessageRejected`, `ProposalRejected`, `ThresholdReached` |
| Execution | `MessageExecuted`, `MessageFailed` |
| Validators | `PrimaryValidatorSet`, `WitnessValidatorAdded`, `ValidatorRemoved`, `ThresholdUpdated` |
| Modules | `ModuleAdded`, `ModuleRemoved` |
| Registry | `MessageTypeRegistered`, `MessageTypeUpdated`, `MessageTypeEnabled` |

> See `PLAN_CONTRACTS.md` Section 8 for event signatures.

### 6.8 Access Control

| Role | Permissions |
|------|-------------|
| `ADMIN_ROLE` | Grant/revoke all roles |
| `REGISTRY_ROLE` | Register/update message types |
| `VALIDATOR_MANAGER_ROLE` | Add/remove validators, set threshold |

Timelock recommended for: threshold changes, validator removal, message type disabling.

### 6.9 TEE Bootstrapping

Validators must complete TEE bootstrapping before signing. Key generation happens inside the enclave, and attestation is submitted during registration.

**Bootstrapping Steps**:
1. Validator starts in TEE, generates signing key inside enclave
2. TEE creates attestation (code hash + key fingerprint)
3. Submit `addValidator(address, attestation)` to Bridge
4. Bridge verifies attestation and adds to signing set
5. Key is already attested; no per-message attestation needed

**Supported TEE Platforms**: GCP Confidential Space, AWS Nitro Enclaves, Azure Confidential VMs, Intel SGX

> See `PLAN_CONTRACTS.md` Section 4 for implementation details.

### 6.10 Bridge Upgrades and TEE Versioning

Bridge uses **UUPS upgradeable proxy** pattern with timelock for admin operations.

**TEE Version Tracking**: Bridge tracks `minimumTeeVersion` and each validator's current version. Old validators must re-attest after upgrades.

| Concern | Mitigation |
|---------|------------|
| Malicious upgrade | Timelock + multisig admin |
| Validator disruption | Grace period for re-attestation |

> See `PLAN_CONTRACTS.md` Section 5 for implementation details.

### 6.11 WETH Handling

Bridge holds **WETH only** (not native ETH) for consistency and reentrancy safety. When executing payable calls, Bridge unwraps WETH to ETH before the call.

**Relayer Reimbursement**: Handled out-of-band (no built-in mechanism).

> See `PLAN_CONTRACTS.md` Section 6 for implementation details.

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

### 8.6 Invariant Security

Invariant checking introduces additional attack surfaces and security considerations. This section covers data source trust, timing attacks, and oracle manipulation.

#### 8.6.1 Data Source Trust Hierarchy

Not all data sources are equally trustworthy. Validators should apply appropriate skepticism based on source:

| Data Source | Trust Level | Manipulation Risk | Mitigation |
|-------------|-------------|-------------------|------------|
| Target chain state (via RPC) | High | Low - protected by blockchain consensus | Use reputable RPC providers |
| Other chain state (cross-chain) | Medium-High | Medium - depends on bridge/RPC | Multiple RPC sources, finality checks |
| Chainlink/Pyth oracles | Medium-High | Medium - oracle manipulation possible | Use TWAP, check multiple feeds |
| Public price APIs | Medium | Medium - API can be compromised | Multiple sources, sanity bounds |
| Application HTTP API | Low | High - application controls response | Verification API pattern, TEE attestation |
| Metadata fields only | None | High - application provides all data | On-chain/oracle verification required |

**Key principle**: Validators should independently verify claims whenever possible. Metadata fields alone should never be trusted for high-value operations.

#### 8.6.2 Stale Data Attacks

**Attack**: Attacker exploits timing gap between data fetch and message execution.

```
1. Validator fetches price at T=0: $100
2. Attacker manipulates market, price drops to $80 at T=1
3. Message executes at T=2 with stale $100 price assumption
4. Attacker profits from price discrepancy
```

**Mitigations**:

| Strategy | Description | Trade-off |
|----------|-------------|-----------|
| Freshness bounds | Reject data older than N seconds | May reject valid messages during high latency |
| Post-execution modules | Re-check state after execution | Reverts cost gas but catch stale data |
| TWAP pricing | Use time-weighted averages | More manipulation-resistant, less responsive |
| Price bounds in calldata | Hard-code acceptable price range | Execution fails if price moves too far |

**Recommended freshness limits**:

| Asset Type | Max Data Age | Rationale |
|------------|--------------|-----------|
| Stablecoins | 5 minutes | Low volatility |
| Major tokens (ETH, BTC) | 1-2 minutes | Moderate volatility |
| Volatile/small-cap | 30 seconds | High volatility |
| Real-time (swaps) | 15 seconds | Price-sensitive |

#### 8.6.3 Oracle Manipulation

**Attack vectors**:

1. **Flash loan attacks**: Attacker borrows large amounts, manipulates on-chain price, executes transaction, repays loan - all in one block
2. **Low liquidity manipulation**: Attacker moves thin markets with relatively small capital
3. **Oracle front-running**: Attacker sees oracle update in mempool, front-runs with stale price

**Mitigations**:

| Attack | Mitigation | Implementation |
|--------|------------|----------------|
| Flash loans | Use TWAP oracles | Chainlink TWAP, Uniswap TWAP |
| Low liquidity | Minimum liquidity requirements | Check pool depth before trusting price |
| Front-running | Post-execution price checks | Module verifies price still valid after execution |
| Single oracle failure | Multiple oracle sources | Require 2-of-3 oracle agreement |

#### 8.6.4 Application Logic Invariant Risks

Application logic invariants carry unique risks because they depend on application-controlled state.

**Risks**:

| Risk | Description | Mitigation |
|------|-------------|------------|
| False state reporting | App lies about gameWinnerCount, balances | Verification API, TEE attestation |
| API unavailability | Verification API goes down | Fallback to trust Primary, or reject |
| API compromise | Attacker controls verification endpoint | mTLS, signed responses, multiple endpoints |
| Race conditions | State changes between check and execution | Idempotency, optimistic locking |

**Trust escalation pattern**:

For applications handling increasing value, progressively add trust mechanisms:

```
Low value ($0-1K):
  └─ Trust Primary validation only

Medium value ($1K-100K):
  └─ Require Verification API
  └─ Multiple Witness validators

High value ($100K+):
  └─ Verification API + App runs in TEE
  └─ App attestation in metadata
  └─ 3-of-5 or higher validator threshold
```

#### 8.6.5 Invariant Failure Modes

All invariant failures result in message rejection. There are no warning or soft-failure modes.

| Failure Type | Validator Behavior | Application Response |
|--------------|-------------------|---------------------|
| On-chain state violation | Reject with `INVARIANT_VIOLATED` | Fix state or adjust parameters, resubmit |
| Oracle data stale | Reject with `INVARIANT_DATA_STALE` | Wait for fresh oracle update, resubmit |
| Oracle data unavailable | Reject with `INVARIANT_DATA_UNAVAILABLE` | Wait for oracle recovery, resubmit |
| Verification API failure | Reject with `INVARIANT_VIOLATED` | Fix app state, resubmit |
| Verification API timeout | Configurable: reject or trust Primary | Ensure API availability |

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

Storage records include: message content (id, type, calldata, metadata, nonce, timestamp, domain, value), Primary Validator signature, and publication metadata.

> See `PLAN_VALIDATORS.md` Section 6 for `StorageRecord` interface.

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

## 10. Appendix

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
        "domain": APP_ID,
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

#### Core Terms

| Term | Definition | Industry Comparison |
|------|------------|---------------------|
| **Message** | The complete data structure submitted by an application for validation and execution | Same as Hyperlane, LayerZero |
| **Message Type** | ABI function signature (e.g., `mint(address,uint256)`) | Similar to function selector |
| **Calldata** | ABI-encoded function parameters executed on-chain. This is the actual data passed to the target contract's function | LayerZero uses "payload"; we use Ethereum-native terminology for precision |
| **Metadata** | Evidence/context provided to convince validators to sign. NOT executed on-chain | Novel concept - no direct industry equivalent |
| **Domain** | Unique identifier for an application using the Bridge. Used for nonce tracking and message attribution | Similar to Hyperlane's "domain" (chain ID) but identifies application, not chain |

#### Validation Terms

| Term | Definition | Industry Comparison |
|------|------------|---------------------|
| **Primary Validator** | Validator connected to application via HTTP, publishes to storage, can initialize messages | Novel - other protocols don't distinguish validator roles |
| **Witness Validator** | Validator that reads from storage, verifies independently, cannot initialize | Novel - provides independent verification |
| **Threshold** | Minimum validator signatures required (M-of-N) | Same as Hyperlane MultisigISM, Gnosis Safe |
| **Schema** | JSON Schema defining required metadata fields | Standard JSON Schema usage |

#### Infrastructure Terms

| Term | Definition | Industry Comparison |
|------|------------|---------------------|
| **Bridge** | Smart contract that aggregates signatures, enforces rules, and executes messages. Can hold assets | Hyperlane: "Mailbox"; LayerZero: "Endpoint". We use "Bridge" as it can hold assets |
| **Module** | Pluggable validation for pre/post execution checks | Similar to Hyperlane ISM; Gnosis Safe calls these "Guards" |
| **Relayer** | Entity that calls `executeMessage()` after threshold is met. Untrusted, permissionless role | Same as Hyperlane "Relayer", LayerZero "Executor" |
| **Storage Layer** | Long-term archival (Arweave, IPFS) for messages and schemas | We distinguish from DA; industry often conflates |
| **DA Layer** | Short-term data availability (Celestia, EigenDA) | Standard industry term |

#### Security Terms

| Term | Definition | Industry Comparison |
|------|------------|---------------------|
| **TEE** | Trusted Execution Environment (GCP Confidential Space, AWS Nitro, etc.) | Standard industry term |
| **Attestation** | Cryptographic proof of TEE integrity, binding signing key to enclave | Standard industry term |
| **Nonce** | Monotonically increasing counter for replay protection | Same as Ethereum transaction nonce |

#### Cross-Reference with Other Protocols

| Our Term | Hyperlane Equivalent | LayerZero Equivalent | IBC Equivalent |
|----------|---------------------|---------------------|----------------|
| Message | Message | Message | Packet |
| Calldata | Message body | Payload | Packet data |
| Bridge | Mailbox | Endpoint | IBC Handler |
| Validator | Validator | DVN | Light Client |
| Relayer | Relayer | Executor | Relayer |
| Module | ISM | Security Stack | - |
| Domain | Domain (chain ID) | eid (endpoint ID) | Channel |
| Threshold | MultisigISM threshold | Required DVNs | - |
