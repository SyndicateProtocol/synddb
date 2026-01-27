# SyndDB Complete Data Flow Documentation

This document describes all data flows between SyndDB components, including message formats, serialization, and concrete examples.

---

## 1. Core Data Flow: Application → Client → Sequencer → Storage → Validator

This is the primary data path for all application state changes.

```
┌─────────────────────────────────────────────────────────────────────────────────────────┐
│                            APPLICATION (App TEE VM1)                                    │
│                                                                                         │
│   ┌─────────────────┐      ┌──────────────────────────────────────────────────────────┐ │
│   │  Your App Code  │      │                  synddb-client Library                   │ │
│   │                 │      │  ┌──────────────────┐   ┌─────────────────────────────┐  │ │
│   │  // App writes  │      │  │ SessionMonitor   │   │ Background Sender Thread    │  │ │
│   │  // to SQLite   │      │  │                  │   │                             │  │ │
│   │  conn.execute(  │ ───► │  │ - Hooks into     │   │ - Buffers changesets        │  │ │
│   │    "INSERT...", │      │  │   SQLite Session │   │ - Batches by size/time      │  │ │
│   │    params       │      │  │ - Captures raw   │──►│ - Sends HTTP POST to        │  │ │
│   │  );             │      │  │   changeset bytes│   │   sequencer                 │  │ │
│   │                 │      │  │ - Tracks sequence│   │ - Retries on failure        │  │ │
│   └─────────────────┘      │  └──────────────────┘   └─────────────┬───────────────┘  │ │
│                            └───────────────────────────────────────┼──────────────────┘ │
└────────────────────────────────────────────────────────────────────┼─────────────────────┘
                                                                     │
                            HTTP POST /changesets (CBOR)             │
                            ChangesetBatchRequest                    │
                                                                     ▼
┌─────────────────────────────────────────────────────────────────────────────────────────┐
│                            SEQUENCER (Sequencer TEE VM2)                                │
│                                                                                         │
│   ┌─────────────────────┐   ┌─────────────────────┐   ┌─────────────────────────────┐   │
│   │     HTTP API        │   │       Inbox         │   │        Batcher              │   │
│   │                     │   │                     │   │                             │   │
│   │ - Receives CBOR     │──►│ - Atomic counter    │──►│ - Groups messages           │   │
│   │ - Validates request │   │ - zstd compression  │   │ - SHA-256 content hash      │   │
│   │ - Returns receipt   │   │ - COSE Sign1 wrap   │   │ - Signs batch               │   │
│   │                     │   │ - secp256k1/keccak  │   │ - Compresses with zstd      │   │
│   └─────────────────────┘   └─────────────────────┘   └─────────────┬───────────────┘   │
│                                                                     │                   │
│                                                                     ▼                   │
│   ┌─────────────────────────────────────────────────────────────────────────────────┐   │
│   │                              Publisher                                          │   │
│   │   ┌─────────────────────────┐     ┌─────────────────────────────────────────┐   │   │
│   │   │ GCS Publisher           │     │ Local Publisher (dev)                   │   │   │
│   │   │ Uploads .cbor.zst       │     │ SQLite storage                          │   │   │
│   │   └───────────┬─────────────┘     └─────────────────────────────────────────┘   │   │
│   └───────────────┼─────────────────────────────────────────────────────────────────┘   │
└───────────────────┼─────────────────────────────────────────────────────────────────────┘
                    │
                    │  gs://{bucket}/batches/000000000001_000000000050.cbor.zst
                    ▼
┌─────────────────────────────────────────────────────────────────────────────────────────┐
│                            GOOGLE CLOUD STORAGE                                         │
│                                                                                         │
│   batches/                                                                              │
│   ├── 000000000001_000000000050.cbor.zst                                                │
│   ├── 000000000051_000000000100.cbor.zst                                                │
│   └── ...                                                                               │
└───────────────────┬─────────────────────────────────────────────────────────────────────┘
                    │
                    │  Fetch and decompress batches
                    ▼
┌─────────────────────────────────────────────────────────────────────────────────────────┐
│                            VALIDATOR                                                    │
│                                                                                         │
│   ┌─────────────────────┐   ┌─────────────────────┐   ┌─────────────────────────────┐   │
│   │   GCS Fetcher       │   │ SignatureVerifier   │   │  ChangesetApplier           │   │
│   │                     │   │                     │   │                             │   │
│   │ - Lists batches     │──►│ - Verifies COSE     │──►│ - Decompresses payload      │   │
│   │ - Downloads .zst    │   │ - Checks sequencer  │   │ - Applies to SQLite         │   │
│   │ - Builds batch index│   │   public key        │   │ - Handles schema changes    │   │
│   └─────────────────────┘   └─────────────────────┘   └─────────────┬───────────────┘   │
│                                                                     │                   │
│                                                                     ▼                   │
│                                                         ┌───────────────────────────┐   │
│                                                         │   Replicated SQLite DB    │   │
│                                                         │   (Identical to App DB)   │   │
│                                                         └───────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────────────────────────┘
```

### Step-by-Step Flow

| Step | Component | Action | Data |
|------|-----------|--------|------|
| 1 | App | Executes SQL: `INSERT INTO trades VALUES (...)` | SQL statement |
| 2 | SQLite | Session Extension captures change | Raw changeset bytes |
| 3 | synddb-client | SessionMonitor extracts changeset | `Changeset { data, sequence, timestamp }` |
| 4 | synddb-client | Background thread batches and sends | `ChangesetBatchRequest` (CBOR) |
| 5 | Sequencer | Assigns sequence, compresses, signs | `CborSignedMessage` |
| 6 | Sequencer | Batcher groups into batch | `CborBatch` with batch signature |
| 7 | Sequencer | Publisher uploads | `000000000042_000000000050.cbor.zst` |
| 8 | Validator | Fetcher downloads batch | Decompressed CBOR |
| 9 | Validator | Verifies sequencer signature | Public key check |
| 10 | Validator | Applies changeset to SQLite | Replicated state |

---

## 2. TEE Bootstrapping Flow

Before sequencers and validators can operate, they must register their signing keys on-chain via the TEE bootstrap process.

```
┌─────────────────────────────────────────────────────────────────────────────────────────┐
│                   SEQUENCER/VALIDATOR (Inside Confidential Space TEE)                   │
│                                                                                         │
│   ┌─────────────────────────────────────────────────────────────────────────────────┐   │
│   │                        BootstrapStateMachine                                    │   │
│   │                                                                                 │   │
│   │   State: NotStarted → GeneratingKey → FetchingAttestation → GeneratingProof    │   │
│   │          → RegisteringKey → VerifyingRegistration → Ready                       │   │
│   └─────────────────────────────────────────────────────────────────────────────────┘   │
│                                                                                         │
│   Step 1: Generate Key                                                                  │
│   ┌─────────────────────────────────────────────────────────────────────────────────┐   │
│   │  EvmKeyManager::generate()                                                      │   │
│   │  - Uses secure OS randomness inside TEE                                         │   │
│   │  - Produces secp256k1 keypair                                                   │   │
│   │  - Output: 64-byte public key, derived 20-byte address                          │   │
│   └─────────────────────────────────────────────────────────────────────────────────┘   │
│                                                                                         │
│   Step 2: Fetch Attestation                                                             │
│   ┌─────────────────────────────────────────────────────────────────────────────────┐   │
│   │  AttestationClient::get_token()                                                 │   │
│   │  - Calls GCP metadata server inside Confidential Space                          │   │
│   │  - Returns JWT with TEE claims (image_digest, secboot, dbgstat)                 │   │
│   └─────────────────────────────────────────────────────────────────────────────────┘   │
└────────────────────────────────────────────────────────────────────────────────────┬────┘
                                                                                     │
                  POST /prove { jwt_token, evm_public_key, image_signature }         │
                                                                                     ▼
┌─────────────────────────────────────────────────────────────────────────────────────────┐
│                            PROOF SERVICE (Cloud Run + GPU)                              │
│                                                                                         │
│   Step 3: Generate ZK Proof                                                             │
│   ┌─────────────────────────────────────────────────────────────────────────────────┐   │
│   │  AttestationProver (RISC Zero zkVM)                                             │   │
│   │                                                                                 │   │
│   │  Inside zkVM guest program:                                                     │   │
│   │  1. Verify RS256 signature on JWT attestation                                   │   │
│   │  2. Validate TEE claims (secboot, dbgstat == "disabled-since-boot")             │   │
│   │  3. Parse image_signature (secp256k1 over image_digest)                         │   │
│   │  4. Derive Ethereum address from public key                                     │   │
│   │  5. Commit PublicValuesStruct to journal                                        │   │
│   │                                                                                 │   │
│   │  Output: Groth16 proof + ABI-encoded public values                              │   │
│   └─────────────────────────────────────────────────────────────────────────────────┘   │
│                                                                                         │
│   Response: { public_values: "0x...", proof_bytes: "0x...", tee_address: "0x..." }      │
└────────────────────────────────────────────────────────────────────────────────────┬────┘
                                                                                     │
                  POST /register-key { public_values, proof_bytes, signature }       │
                                                                                     ▼
┌─────────────────────────────────────────────────────────────────────────────────────────┐
│                            RELAYER (Cloud Run)                                          │
│                                                                                         │
│   Step 4: Register Key                                                                  │
│   ┌─────────────────────────────────────────────────────────────────────────────────┐   │
│   │  RelayerClient                                                                  │   │
│   │                                                                                 │   │
│   │  1. TEE signs EIP-712 registration request:                                     │   │
│   │     AddKey(bytes32 attestationHash, uint256 deadline)                           │   │
│   │     attestationHash = keccak256(publicValues || proofBytes)                     │   │
│   │                                                                                 │   │
│   │  2. Sends to relayer with signature                                             │   │
│   │  3. Relayer pays gas, submits to TeeKeyManager contract                         │   │
│   └─────────────────────────────────────────────────────────────────────────────────┘   │
└────────────────────────────────────────────────────────────────────────────────────┬────┘
                                                                                     │
                  Transaction: TeeKeyManager.addKey(proof, publicValues, signature)  │
                                                                                     ▼
┌─────────────────────────────────────────────────────────────────────────────────────────┐
│                            BLOCKCHAIN (L1)                                              │
│                                                                                         │
│   ┌─────────────────────────────────────────────────────────────────────────────────┐   │
│   │  TeeKeyManager Contract                                                         │   │
│   │                                                                                 │   │
│   │  1. Verify RISC Zero proof against known image ID                               │   │
│   │  2. Verify image_signature via ecrecover → must match authorized signer         │   │
│   │  3. Verify EIP-712 signature from TEE key                                       │   │
│   │  4. Register key: sequencerKeys[address] = true or validatorKeys[address] = true│   │
│   └─────────────────────────────────────────────────────────────────────────────────┘   │
│                                                                                         │
│   Step 5: Verify Registration                                                           │
│   ┌─────────────────────────────────────────────────────────────────────────────────┐   │
│   │  ContractSubmitter.is_sequencer_key_valid(address)                              │   │
│   │  - Queries contract to confirm key is registered                                │   │
│   │  - Retries to handle RPC indexing delay                                         │   │
│   └─────────────────────────────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────────────────────────┘
```

### Bootstrap Data Structures

#### ProofRequest (to Proof Service)
```rust
struct ProofRequest {
    jwt_token: String,           // Raw JWT from Confidential Space
    expected_audience: String,   // Audience claim to verify
    evm_public_key: String,      // 64-byte hex, 0x-prefixed
    image_signature: String,     // 65-byte hex (r||s||v), 0x-prefixed
}
```

#### ProofResponse (from Proof Service)
```rust
struct ProofResponse {
    public_values: String,       // ABI-encoded PublicValuesStruct, 0x-prefixed
    proof_bytes: String,         // Groth16 proof, 0x-prefixed
    tee_address: String,         // Derived address for verification
}
```

#### PublicValuesStruct (committed by zkVM)
```rust
struct PublicValuesStruct {
    jwk_key_hash: [u8; 32],           // keccak256(signing_key_id)
    validity_window_start: u64,       // JWT nbf claim
    validity_window_end: u64,         // JWT exp claim
    image_digest_hash: [u8; 32],      // keccak256(image_digest)
    tee_signing_key: Address,         // Derived from public key
    secboot: bool,                    // Secure boot enabled
    dbgstat_disabled: bool,           // Debug disabled since boot
    audience_hash: [u8; 32],          // keccak256(audience)
    image_signature_v: u8,            // For ecrecover
    image_signature_r: [u8; 32],      // For ecrecover
    image_signature_s: [u8; 32],      // For ecrecover
}
```

#### RegisterKeyRequest (to Relayer)
```json
{
  "publicValues": "0x...",
  "proofBytes": "0x...",
  "deadline": 1700003600,
  "signature": "0x...",
  "keyType": "sequencer"
}
```

---

## 3. Bridge Message Flow: Sequencer → Relayer → Bridge → Target Contract

This section details how messages flow from the sequencer through the Bridge contract to target contracts (e.g., PriceOracle). The Bridge requires:
1. **Sequencer signature** - Proves the message was sequenced by a registered TEE
2. **Validator signatures** - Additional attestations that meet a configurable threshold

```
┌─────────────────────────────────────────────────────────────────────────────────────────┐
│                            SEQUENCER                                                    │
│                                                                                         │
│   After batching messages, sequencer also creates Bridge-compatible signatures:         │
│                                                                                         │
│   ┌─────────────────────────────────────────────────────────────────────────────────┐   │
│   │                          Inbox / Batcher                                        │   │
│   │                                                                                 │   │
│   │  For each message requiring on-chain execution:                                 │   │
│   │                                                                                 │   │
│   │  1. Compute messageHash:                                                        │   │
│   │     messageHash = keccak256(messageId || targetAddress || keccak256(payload)    │   │
│   │                             || nativeTokenAmount)                               │   │
│   │                                                                                 │   │
│   │  2. Sign with EIP-191:                                                          │   │
│   │     ethSignedHash = keccak256("\x19Ethereum Signed Message:\n32" + messageHash) │   │
│   │     signature = secp256k1_sign(ethSignedHash, sequencer_private_key)            │   │
│   │                                                                                 │   │
│   │  3. Return SequencerSignature { signature, submittedAt }                        │   │
│   └─────────────────────────────────────────────────────────────────────────────────┘   │
│                                                                                         │
│   Two parallel outputs:                                                                 │
│   ├─► Batch to GCS (for validator sync)                                                 │
│   └─► SequencerSignature to Relayer (for Bridge initialization)                         │
└───────────────────────────┬─────────────────────────────────────────────────────────────┘
                            │
                            │  POST /bridge/messages { messageId, target, payload,
                            │                          sequencerSignature, nativeAmount }
                            ▼
┌─────────────────────────────────────────────────────────────────────────────────────────┐
│                            RELAYER                                                      │
│                                                                                         │
│   Step 1: Initialize message on Bridge                                                  │
│   ┌─────────────────────────────────────────────────────────────────────────────────┐   │
│   │  Bridge.initializeMessage(                                                      │   │
│   │      messageId,                    // bytes32 unique ID                         │   │
│   │      targetAddress,                // e.g., PriceOracle contract                │   │
│   │      payload,                      // abi.encodeCall(PriceOracle.updatePrice,   │   │
│   │                                    //                ("BTC", 50000e8, timestamp))│   │
│   │      SequencerSignature {          // From sequencer                            │   │
│   │          signature,                // 65-byte ECDSA (r||s||v)                   │   │
│   │          submittedAt               // Block timestamp                           │   │
│   │      },                                                                         │   │
│   │      nativeTokenAmount             // ETH to transfer with call                 │   │
│   │  )                                                                              │   │
│   └─────────────────────────────────────────────────────────────────────────────────┘   │
│                                                                                         │
│   Step 2: Collect validator signatures                                                  │
│   ┌─────────────────────────────────────────────────────────────────────────────────┐   │
│   │  For each validator:                                                            │   │
│   │    GET validator_url/signature/{messageId}                                      │   │
│   │    → Returns MessageSignature { message_id, signature, signer, signed_at }      │   │
│   │                                                                                 │   │
│   │  Then submit each to Bridge:                                                    │   │
│   │    Bridge.signMessageWithSignature(messageId, validatorSignature)               │   │
│   └─────────────────────────────────────────────────────────────────────────────────┘   │
│                                                                                         │
│   Step 3: Execute message                                                               │
│   ┌─────────────────────────────────────────────────────────────────────────────────┐   │
│   │  Bridge.handleMessage(messageId)                                                │   │
│   │  // Or use initializeAndHandleMessage() to do all in one transaction            │   │
│   └─────────────────────────────────────────────────────────────────────────────────┘   │
└───────────────────────────┬─────────────────────────────────────────────────────────────┘
                            │
                            │  Transaction to Bridge contract
                            ▼
┌─────────────────────────────────────────────────────────────────────────────────────────┐
│                            BRIDGE CONTRACT (L1)                                         │
│                                                                                         │
│   initializeMessage():                                                                  │
│   ┌─────────────────────────────────────────────────────────────────────────────────┐   │
│   │  1. Check message not already initialized                                       │   │
│   │  2. Verify sequencer signature:                                                 │   │
│   │     - Recover signer from EIP-191 signature                                     │   │
│   │     - Check teeKeyManager.isKeyValid(KeyType.Sequencer, signer)                 │   │
│   │  3. Store MessageState { messageId, targetAddress, PreExecution, payload, ... } │   │
│   │  4. Emit MessageInitialized(messageId, payload)                                 │   │
│   └─────────────────────────────────────────────────────────────────────────────────┘   │
│                                                                                         │
│   signMessageWithSignature():                                                           │
│   ┌─────────────────────────────────────────────────────────────────────────────────┐   │
│   │  1. Recover validator address from signature                                    │   │
│   │  2. Check teeKeyManager.isKeyValid(KeyType.Validator, validator)                │   │
│   │  3. Record validatorSignatures[messageId][validator] = true                     │   │
│   │  4. Emit MessageSigned(messageId, validator)                                    │   │
│   └─────────────────────────────────────────────────────────────────────────────────┘   │
│                                                                                         │
│   handleMessage():                                                                      │
│   ┌─────────────────────────────────────────────────────────────────────────────────┐   │
│   │  1. Validate message is in PreExecution stage                                   │   │
│   │  2. Run pre-execution modules (e.g., ValidatorSignatureModule checks threshold) │   │
│   │  3. Unwrap native tokens if nativeTokenAmount > 0                               │   │
│   │  4. Execute: targetAddress.call{value: amount}(payload)  ─────────────────────┐ │   │
│   │  5. Run post-execution modules                                                 │ │   │
│   │  6. Mark message as Completed                                                  │ │   │
│   │  7. Emit MessageHandled(messageId, success)                                    │ │   │
│   └────────────────────────────────────────────────────────────────────────────────┼─┘   │
└────────────────────────────────────────────────────────────────────────────────────┼─────┘
                                                                                     │
                            targetAddress.call(payload)                              │
                                                                                     ▼
┌─────────────────────────────────────────────────────────────────────────────────────────┐
│                            TARGET CONTRACT (e.g., PriceOracle)                          │
│                                                                                         │
│   Receives call from Bridge:                                                            │
│   ┌─────────────────────────────────────────────────────────────────────────────────┐   │
│   │  // payload = abi.encodeCall(PriceOracle.updatePrice, ("BTC", 50000e8, ts))     │   │
│   │                                                                                 │   │
│   │  function updatePrice(                                                          │   │
│   │      string calldata asset,     // "BTC"                                        │   │
│   │      uint256 price,             // 50000_00000000 (8 decimals)                  │   │
│   │      uint256 timestamp          // Unix timestamp from off-chain                │   │
│   │  ) external onlyRole(UPDATER_ROLE) {                                            │   │
│   │      // msg.sender == Bridge contract (has UPDATER_ROLE)                        │   │
│   │      prices[assetHash(asset)] = Price(price, timestamp, block.timestamp);       │   │
│   │      emit PriceUpdated(asset, price, timestamp, msg.sender);                    │   │
│   │  }                                                                              │   │
│   └─────────────────────────────────────────────────────────────────────────────────┘   │
│                                                                                         │
│   Result: On-chain state updated based on off-chain TEE computation                     │
└─────────────────────────────────────────────────────────────────────────────────────────┘
```

### Bridge Message Lifecycle

| Stage | Description |
|-------|-------------|
| `NotStarted` | Message ID not yet used |
| `PreExecution` | Initialized, awaiting pre-execution module validation |
| `Executing` | Currently executing target contract call |
| `PostExecution` | Execution complete, running post-execution modules |
| `Completed` | Successfully processed |
| `Rejected` | Rejected by validation module |

### Bridge Contract Data Structures

#### SequencerSignature
```solidity
struct SequencerSignature {
    bytes signature;       // 65-byte ECDSA (r || s || v)
    uint256 submittedAt;   // Block timestamp when submitted
}
```

#### MessageState
```solidity
struct MessageState {
    bytes32 messageId;
    address targetAddress;
    ProcessingStage stage;
    bytes payload;
    uint256 createdAt;
    uint256 nativeTokenAmount;
}
```

### Bridge API Calls

**initializeMessage** - Submit sequencer-signed message
```solidity
function initializeMessage(
    bytes32 messageId,
    address targetAddress,
    bytes calldata payload,
    SequencerSignature calldata sequencerSignature,
    uint256 nativeTokenAmount
) external;
```

**signMessageWithSignature** - Submit validator signature
```solidity
function signMessageWithSignature(
    bytes32 messageId,
    bytes calldata signature  // 65-byte ECDSA from validator
) external;
```

**handleMessage** - Execute the message
```solidity
function handleMessage(bytes32 messageId) external;
```

**initializeAndHandleMessage** - All-in-one (init + validator sigs + execute)
```solidity
function initializeAndHandleMessage(
    bytes32 messageId,
    address targetAddress,
    bytes calldata payload,
    SequencerSignature calldata sequencerSignature,
    bytes[] calldata validatorSignatures,
    uint256 nativeTokenAmount
) external;
```

---

## 3.1 Inbound Message Flow: Bridge → Chain Monitor → Application

Inbound messages (e.g., deposits) flow FROM the blockchain INTO SyndDB. This is the reverse direction of outbound bridge messages.

```
┌─────────────────────────────────────────────────────────────────────────────────────────┐
│                            BLOCKCHAIN (L1)                                              │
│                                                                                         │
│   User deposits ETH/tokens to Bridge:                                                   │
│   ┌─────────────────────────────────────────────────────────────────────────────────┐   │
│   │  Bridge.receive() or Bridge.depositTokens(amount, recipient, data)              │   │
│   │                                                                                 │   │
│   │  Emits: Deposit(from, to, amount, data)                                         │   │
│   │         NativeTokenWrapped(sender, amount)                                      │   │
│   └─────────────────────────────────────────────────────────────────────────────────┘   │
└────────────────────────────────────────────────────────────────────────────────────┬────┘
                                                                                     │
                  WebSocket subscription to Bridge contract events                   │
                                                                                     ▼
┌─────────────────────────────────────────────────────────────────────────────────────────┐
│                            CHAIN MONITOR                                                │
│                                                                                         │
│   ┌─────────────────────────────────────────────────────────────────────────────────┐   │
│   │  ChainMonitor                                                                   │   │
│   │                                                                                 │   │
│   │  1. WebSocket connects to RPC node(s)                                           │   │
│   │  2. Subscribes to logs for Bridge contract address                              │   │
│   │  3. Filters by event signature (e.g., Deposit topic)                            │   │
│   │  4. Waits for confirmation depth (e.g., 12 blocks)                              │   │
│   │  5. Dispatches to registered MessageHandler                                     │   │
│   └─────────────────────────────────────────────────────────────────────────────────┘   │
│                                                                                         │
│   Event Log received:                                                                   │
│   ┌─────────────────────────────────────────────────────────────────────────────────┐   │
│   │  Log {                                                                          │   │
│   │      address: 0x1234...Bridge,                                                  │   │
│   │      topics: [keccak256("Deposit(address,address,uint256,bytes)"),              │   │
│   │               from_indexed, to_indexed],                                        │   │
│   │      data: abi.encode(amount, data),                                            │   │
│   │      block_number: 12345678,                                                    │   │
│   │      transaction_hash: 0xabcd...,                                               │   │
│   │  }                                                                              │   │
│   └─────────────────────────────────────────────────────────────────────────────────┘   │
└────────────────────────────────────────────────────────────────────────────────────┬────┘
                                                                                     │
                  MessageHandler.handle_event(log)                                   │
                                                                                     ▼
┌─────────────────────────────────────────────────────────────────────────────────────────┐
│                            APPLICATION (App TEE)                                        │
│                                                                                         │
│   ┌─────────────────────────────────────────────────────────────────────────────────┐   │
│   │  DepositHandler (implements MessageHandler)                                     │   │
│   │                                                                                 │   │
│   │  async fn handle_event(&self, log: &Log) -> Result<bool> {                      │   │
│   │      // 1. Decode event data                                                    │   │
│   │      let (from, to, amount, data) = decode_deposit_event(log)?;                 │   │
│   │                                                                                 │   │
│   │      // 2. Credit user in application database                                  │   │
│   │      conn.execute(                                                              │   │
│   │          "INSERT INTO balances (user, amount) VALUES (?, ?)                     │   │
│   │           ON CONFLICT(user) DO UPDATE SET amount = amount + ?",                 │   │
│   │          (to, amount, amount)                                                   │   │
│   │      )?;                                                                        │   │
│   │                                                                                 │   │
│   │      // 3. Record deposit for audit trail                                       │   │
│   │      conn.execute(                                                              │   │
│   │          "INSERT INTO deposits (tx_hash, from_addr, to_addr, amount, block)     │   │
│   │           VALUES (?, ?, ?, ?, ?)",                                              │   │
│   │          (log.transaction_hash, from, to, amount, log.block_number)             │   │
│   │      )?;                                                                        │   │
│   │                                                                                 │   │
│   │      Ok(true)  // Event handled successfully                                    │   │
│   │  }                                                                              │   │
│   └─────────────────────────────────────────────────────────────────────────────────┘   │
│                                                                                         │
│   SQLite Session Extension captures changes automatically                               │
└────────────────────────────────────────────────────────────────────────────────────┬────┘
                                                                                     │
                  Changeset captured → Core Data Flow (Section 1)                    │
                                                                                     ▼
┌─────────────────────────────────────────────────────────────────────────────────────────┐
│                            synddb-client → Sequencer → GCS → Validator                  │
│                                                                                         │
│   The deposit-triggered state change now follows the standard outbound flow:            │
│   - Client batches changeset                                                            │
│   - Sequencer signs and sequences                                                       │
│   - Batch published to GCS                                                              │
│   - Validator syncs and replicates state                                                │
└─────────────────────────────────────────────────────────────────────────────────────────┘
```

### Inbound Message Types

| Event | Description | Handler Action |
|-------|-------------|----------------|
| `Deposit(from, to, amount, data)` | User deposited ETH/tokens | Credit user balance |
| `NativeTokenWrapped(sender, amount)` | ETH wrapped to WETH | Track bridge liquidity |
| `KeyRegistered(keyType, publicKey)` | New TEE key registered | Update authorized signers |
| `MessageHandled(messageId, success)` | Outbound message executed | Mark withdrawal complete |

### Chain Monitor Configuration

```rust
pub struct ChainMonitorConfig {
    pub ws_urls: Vec<String>,           // WebSocket RPC endpoints
    pub contract_address: Address,       // Bridge contract to monitor
    pub start_block: u64,               // Block to start from
    pub confirmation_blocks: u64,        // Blocks to wait before processing
    pub event_signatures: Vec<B256>,     // Filter specific events (optional)
    pub reconnect_interval: Duration,    // WebSocket reconnect delay
}
```

### MessageHandler Trait

```rust
#[async_trait]
pub trait MessageHandler: Send + Sync {
    /// Process an event log, return true if handled
    async fn handle_event(&self, log: &Log) -> Result<bool>;

    /// Event signature to filter (None = all events)
    fn event_signature(&self) -> Option<B256>;

    /// Called when monitor starts
    async fn on_start(&self) -> Result<()>;

    /// Called when monitor stops
    async fn on_stop(&self) -> Result<()>;
}
```

---

## 4. System Architecture Overview

```
                                    ┌──────────────────────────────────────┐
                                    │         Blockchain (L1)              │
                                    │  ┌─────────────────────────────────┐ │
                                    │  │   Bridge + TeeKeyManager        │ │
                                    │  │   - Key registration            │ │
                                    │  │   - Deposit events              │ │
                                    │  │   - Withdrawal execution        │ │
                                    │  └─────────────────────────────────┘ │
                                    └───────────────────┬──────────────────┘
                                                        │
        ┌───────────────────────────────────────────────┼───────────────────────────────────────────────┐
        │                                               │                                               │
        ▼                                               ▼                                               ▼
┌───────────────────┐                      ┌─────────────────────────┐                      ┌───────────────────┐
│  Proof Service    │                      │        Relayer          │                      │   Chain Monitor   │
│  (Cloud Run+GPU)  │                      │     (Cloud Run)         │                      │                   │
│                   │                      │                         │                      │ - WebSocket sub   │
│ - RISC Zero zkVM  │                      │ - Bootstrap gas sponsor │                      │ - Event filtering │
│ - Groth16 proofs  │                      │ - Withdrawal submitter  │                      │ - Handler dispatch│
└─────────┬─────────┘                      └────────────┬────────────┘                      └─────────┬─────────┘
          │                                             │                                             │
          │ POST /prove                                 │ POST /register-key                          │ Events
          │                                             │ GET /signature/{id}                         │
          ▼                                             ▼                                             ▼
┌─────────────────────────────────────────────────────────────────────────────────────────────────────────────────┐
│                                            APPLICATION (App TEE)                                                │
│  ┌─────────────────────────────────────────────────────────────────────────────────────────────────────────┐    │
│  │                                          synddb-client                                                  │    │
│  │  - Bootstrap: generates key, fetches attestation, requests proof, registers via relayer                 │    │
│  │  - Runtime: captures changesets, sends to sequencer                                                     │    │
│  └──────────────────────────────────────────────────────────────────────────────────────────────┬──────────┘    │
└─────────────────────────────────────────────────────────────────────────────────────────────────┼───────────────┘
                                                                                                  │
                                              HTTP POST /changesets, /withdrawals                 │
                                                                                                  ▼
┌─────────────────────────────────────────────────────────────────────────────────────────────────────────────────┐
│                                            SEQUENCER (Sequencer TEE)                                            │
│  ┌─────────────────────────────────────────────────────────────────────────────────────────────────────────┐    │
│  │  - Bootstrap: same process as client                                                                    │    │
│  │  - Runtime: atomic sequencing, COSE signing, batching, publishing to GCS                                │    │
│  └──────────────────────────────────────────────────────────────────────────────────────────────┬──────────┘    │
└─────────────────────────────────────────────────────────────────────────────────────────────────┼───────────────┘
                                                                                                  │
                                              .cbor.zst batches to GCS                            │
                                                                                                  ▼
                                              ┌─────────────────────────────────────────┐
                                              │         Google Cloud Storage            │
                                              └───────────────────────┬─────────────────┘
                                                                      │
                                              Fetch batches           │
                                                                      ▼
┌─────────────────────────────────────────────────────────────────────────────────────────────────────────────────┐
│                                            VALIDATOR                                                            │
│  ┌─────────────────────────────────────────────────────────────────────────────────────────────────────────┐    │
│  │  - Bootstrap: same process as sequencer/client                                                          │    │
│  │  - Runtime: fetches batches, verifies signatures, applies changesets, signs withdrawals                 │    │
│  │  - Signature API: serves withdrawal signatures to relayer                                               │    │
│  └─────────────────────────────────────────────────────────────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────────────────────────────────────────────────────────────┘
```

---

## 5. Client → Sequencer Request/Response Formats

### 5.1 Changeset Submission

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

### 5.2 Withdrawal Submission

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

### 5.3 Snapshot Submission

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

## 6. Sequencer Internal Data Flow

### 6.1 Inbox Sequencing

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

### 6.2 COSE Message Structure

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

### 6.3 Batch Creation

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

## 7. Sequencer → Storage Data Flow

### 7.1 GCS Publisher

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

### 7.2 Local Publisher

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

## 8. Storage → Validator Data Flow

### 8.1 HTTP Fetcher

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

### 8.2 GCS Fetcher (Batch Mode)

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

## 9. Validator Internal Data Flow

### 9.1 Signature Verification

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

### 9.2 Changeset Application

After verification:
1. Decompress payload with zstd
2. If `MessageType::Changeset`: apply SQLite changeset
3. If `MessageType::Snapshot`: replace database, reapply pending changesets
4. If `MessageType::Withdrawal`: trigger signing callback
5. Update `StateStore` with new sequence

### 9.3 Bridge Signer (Optional)

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

## 10. Validator → Relayer Data Flow (Withdrawals)

### 10.1 Signature Retrieval API

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

### 10.2 List Pending Signatures

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

### 10.3 Signer Info

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

## 11. Chain Monitor → Application Data Flow

> **See also**: Section 3.1 for the complete inbound message flow diagram showing how blockchain events flow through the Chain Monitor into the application and then into the SyndDB replication pipeline.

### 11.1 Blockchain Events

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

## 12. Message Queue API (Sequencer)

### 12.1 Inbound Messages

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

### 12.2 Acknowledgement

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

## 13. Status and Monitoring APIs

### 13.1 Sequencer Status

**Endpoint**: `GET /status`

**Response**: `StatusResponse`
```json
{
  "current_sequence": 42,
  "signer_address": "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
  "signer_pubkey": "0x83185678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678"
}
```

### 13.2 Validator Status

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

### 13.3 Batch Statistics

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

## 14. Serialization Reference

### 14.1 Binary Fields

In **JSON**: Binary fields are base64-encoded
```json
{"data": "SGVsbG8gV29ybGQ="}
```

In **CBOR**: Binary fields are raw bytes (major type 2)

### 14.2 Signature Formats

| Format | Length | Description |
|--------|--------|-------------|
| COSE signature | 64 bytes | `r || s` (no recovery ID) |
| EIP-191 signature | 65 bytes | `r || s || v` (Bridge-compatible) |
| Public key | 64 bytes | Uncompressed secp256k1 (no 0x04 prefix) |
| Address | 20 bytes | keccak256(pubkey)[12:32] |

### 14.3 Hash Functions

| Purpose | Algorithm |
|---------|-----------|
| Message hash | keccak256 |
| Batch content hash | SHA-256 |
| Signature digest | keccak256 |

---

## 15. Complete Flow Example

### Deposit (Inbound) → Application State → Withdrawal (Outbound)

This example shows both directions of the data flow.

**Part A: Inbound Flow (Deposit)**
```
1. User deposits ETH to Bridge contract on L1
   ↓
2. Bridge emits Deposit(from, to, amount, data) event
   ↓
3. chain-monitor captures event via WebSocket (Section 3.1)
   ↓
4. DepositHandler.handle_event() processes deposit
   ↓
5. Handler credits user in application SQLite database
   ↓
6. SQLite Session Extension captures changeset
   ↓
7. synddb-client batches changeset, sends POST /changesets (Section 1)
   ↓
8. synddb-sequencer:
   - Assigns sequence number
   - Compresses with zstd
   - Signs with COSE/secp256k1
   - Returns SequenceResponse
   ↓
9. Batcher groups messages into batch
   ↓
10. Publisher uploads .cbor.zst to GCS
    ↓
11. synddb-validator fetches batch from GCS
    ↓
12. Validator verifies sequencer signature
    ↓
13. Validator applies changesets to replica database
    ↓
    User's deposit is now reflected in both app and validator state
```

**Part B: Outbound Flow (Withdrawal via Bridge)**
```
14. User requests withdrawal via application
    ↓
15. Application triggers bridge message (withdrawal is just one message type)
    ↓
16. Sequencer creates message with EIP-191 signature (Section 3)
    ↓
17. Parallel paths:
    ├─► Batch to GCS (for validator sync)
    └─► Sequencer signature to Relayer
    ↓
18. Relayer calls Bridge.initializeMessage(messageId, target, payload, sig, amount)
    ↓
19. Validator syncs message, creates EIP-191 signature
    ↓
20. Relayer polls validator GET /pending, GET /signature/{id}
    ↓
21. Relayer calls Bridge.signMessageWithSignature(messageId, validatorSig)
    ↓
22. Once threshold met, Relayer calls Bridge.handleMessage(messageId)
    ↓
23. Bridge executes: targetAddress.call{value: amount}(payload)
    ↓
24. For withdrawals: ETH transferred to recipient
    For other messages: Target contract function executed
```

---

## 16. Environment Variables Reference

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

### TEE Bootstrap (Sequencer/Validator)
| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `PROOF_SERVICE_URL` | Yes | - | URL of proof generation service |
| `RELAYER_URL` | Yes | - | URL of key registration relayer |
| `BRIDGE_ADDRESS` | Yes | - | Bridge contract address |
| `RPC_URL` | Yes | - | Blockchain RPC endpoint |
| `CHAIN_ID` | Yes | - | Target chain ID |
| `IMAGE_SIGNATURE` | Yes | - | 65-byte secp256k1 sig over image digest |
| `ATTESTATION_AUDIENCE` | No | auto | Expected JWT audience |
| `PROVER_MODE` | No | `service` | `service` or `mock` |

### Proof Service
| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `BIND_ADDRESS` | No | `0.0.0.0:8080` | HTTP bind address |
| `GOOGLE_OIDC_DISCOVERY_URL` | No | GCP default | OIDC discovery endpoint |
| `JWKS_CACHE_TTL_SECS` | No | `3600` | JWK cache TTL |
| `LOG_JSON` | No | `false` | JSON logging format |

### Relayer
| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `SIGNING_KEY` | Yes | - | Relayer's private key (pays gas) |
| `RPC_URL` | Yes | - | Blockchain RPC endpoint |
| `BRIDGE_ADDRESS` | Yes | - | Bridge contract address |
| `CHAIN_ID` | Yes | - | Target chain ID |
