# Bridge Contract Implementation Plan

> Implementation details for the Message Passing Bridge smart contracts.
> See `SPEC.md` for the specification.

## Overview

This document contains implementation details, code patterns, and Solidity examples for the Bridge contract system. The Bridge is the trust anchor that:

- Maintains the message type registry with schema hashes
- Tracks application domains and their Primary Validators
- Aggregates validator signatures with threshold enforcement
- Executes validated messages via modular pre/post hooks
- Manages nonces, expiration, and rejection flows

## 1. Contract Architecture

```
contracts/
├── MessageBridge.sol              # Main bridge contract (UUPS upgradeable)
├── interfaces/
│   ├── IMessageBridge.sol         # Core bridge interface
│   ├── IMessageTypeRegistry.sol   # Message type registration
│   └── IModule.sol                # Module interface
├── modules/
│   ├── RateLimitModule.sol        # Global rate limiting
│   ├── AmountThresholdModule.sol  # Flag large transfers
│   ├── AllowlistModule.sol        # Restrict to known addresses
│   ├── TimelockModule.sol         # Delay sensitive operations
│   └── SupplyCapModule.sol        # Post-execution supply check
└── libraries/
    ├── SignatureVerifier.sol      # EIP-712 signature verification
    └── NonceManager.sol           # Per-domain nonce tracking
```

## 2. Constants

```solidity
// Timing
uint256 public constant MAX_CLOCK_DRIFT = 60;          // 1 minute tolerance for timestamps
uint256 public constant DEFAULT_EXPIRATION = 86400;    // 24 hours
uint256 public constant UPGRADE_DELAY = 2 days;        // Timelock for upgrades

// Limits
uint256 public constant MAX_BATCH_SIZE = 50;           // Max messages per batch operation
uint256 public constant MAX_MODULES_PER_TYPE = 10;     // Max modules per message type
uint256 public constant MAX_STORAGE_REF_LENGTH = 500;  // Max length for storageRef URIs

// Signatures
uint256 public constant MIN_THRESHOLD = 1;             // Minimum signature threshold
```

## 3. Custom Errors

```solidity
// Authorization
error NotPrimaryValidator(bytes32 domain, address caller);
error NotValidator(address caller);
error NotAuthorized(address caller, bytes32 role);

// Message State
error MessageNotInitialized(bytes32 messageId);
error MessageAlreadyInitialized(bytes32 messageId);
error MessageNotPending(bytes32 messageId, MessageStage currentStage);
error MessageNotReady(bytes32 messageId, MessageStage currentStage);
error MessageAlreadyTerminal(bytes32 messageId, MessageStage currentStage);

// Nonce & Expiration
error InvalidNonce(bytes32 domain, uint64 expected, uint64 provided);
error MessageExpired(bytes32 messageId, uint256 expiredAt);
error TimestampOutOfRange(uint64 timestamp, uint256 currentTime, uint256 maxDrift);

// Signatures & Rejections
error AlreadySigned(bytes32 messageId, address validator);
error AlreadyRejected(bytes32 messageId, address validator);
error ProposalAlreadyRejected(bytes32 messageId);
error InvalidSignature(bytes32 messageId, address recoveredSigner);

// Execution
error ExecutionFailed(bytes32 messageId, bytes returnData);
error ModuleCheckFailed(bytes32 messageId, address module, string reason);
error InsufficientWETHBalance(uint256 required, uint256 available);

// Configuration
error MessageTypeNotRegistered(string messageType);
error MessageTypeDisabled(string messageType);
error ApplicationNotRegistered(bytes32 domain);
error ApplicationNotActive(bytes32 domain);
error InvalidThreshold(uint256 threshold, uint256 validatorCount);

// Validation
error InvalidAddress(string param);
error InvalidAttestation(address validator);
error VersionBelowMinimum(string provided, string minimum);
error BatchSizeExceeded(uint256 provided, uint256 maximum);
error StorageRefTooLong(uint256 provided, uint256 maximum);
```

## 4. Core Data Structures

### 4.1 Message State Machine

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
    string messageType;           // ABI signature (e.g., "mint(address,uint256)")
    bytes calldata_;              // ABI-encoded function parameters
    bytes32 metadataHash;         // keccak256 of canonical metadata JSON
    string storageRef;            // URI(s) to full metadata (ar://...|ipfs://...)
    uint256 value;                // Native token amount (in WETH)
    uint64 nonce;                 // Per-domain sequential nonce
    uint64 timestamp;             // Message creation timestamp
    bytes32 domain;               // Application identifier
    address primaryValidator;     // Who initialized this message
    uint256 signaturesCollected;  // Count of valid signatures
    uint256 rejectionsCollected;  // Count of rejections (informational)
}

mapping(bytes32 => MessageState) public messageStates;
mapping(bytes32 => mapping(address => bool)) public hasValidatorSigned;
mapping(bytes32 => mapping(address => bool)) public hasValidatorRejected;
```

### 2.2 Application Configuration

```solidity
struct ApplicationConfig {
    address primaryValidator;       // Who can initialize messages for this domain
    uint64 expirationSeconds;       // Message expiration (default: 86400 = 24h)
    bool requireWitnessSignatures;  // If true, threshold > 1 required
    bool active;                    // Whether application is currently active
}

mapping(bytes32 => ApplicationConfig) public applicationConfigs;
mapping(bytes32 => uint64) public lastNonce;  // domain -> last consumed nonce
```

### 2.3 Validator Info

```solidity
struct ValidatorInfo {
    address validator;
    bool active;
    bool isWitness;               // false = Primary (domain-specific), true = Witness (global)
    uint64 registeredAt;
    bytes32 teeCodeHash;          // Hash of validator code running in TEE
    string teeVersion;            // Semantic version (e.g., "1.2.3")
    uint64 lastAttestationAt;     // When attestation was last verified
}

mapping(address => ValidatorInfo) public validatorInfo;
EnumerableSet.AddressSet private witnessValidators;
string public minimumTeeVersion;
uint256 public signatureThreshold;  // M-of-N required signatures
```

### 2.4 Rejection Tracking

```solidity
struct Rejection {
    address validator;
    bytes32 reasonHash;           // keccak256 of rejection reason JSON
    string reasonRef;             // Storage reference to full reason
    uint64 timestamp;
}

// Rejections stored separately (a message can have rejections AND still execute)
mapping(bytes32 => Rejection[]) public messageRejections;

// Track rejected proposals (messages rejected before initialization)
mapping(bytes32 => bool) public proposalRejected;
```

### 2.5 Message Type Configuration

```solidity
struct MessageTypeConfig {
    bytes4 selector;              // bytes4(keccak256(messageType))
    address target;               // Contract to call when executing
    bytes32 schemaHash;           // keccak256 of metadata JSON Schema
    string schemaUri;             // Where to fetch full schema (IPFS, Arweave)
    bool enabled;                 // Whether currently active
    uint64 updatedAt;             // Last update timestamp
}

mapping(string => MessageTypeConfig) public messageTypes;
mapping(bytes4 => string) public selectorToMessageType;
```

## 3. Application & Domain Management

### 3.1 Application Registration

```solidity
function registerApplication(
    bytes32 domain,
    ApplicationConfig calldata config
) external onlyRole(REGISTRY_ROLE) {
    require(applicationConfigs[domain].primaryValidator == address(0), "Already registered");
    require(config.primaryValidator != address(0), "Invalid primary");
    require(validatorInfo[config.primaryValidator].active, "Primary not registered as validator");

    applicationConfigs[domain] = config;
    lastNonce[domain] = 0;

    emit ApplicationRegistered(domain, config.primaryValidator, config.expirationSeconds);
}

function updateApplicationConfig(
    bytes32 domain,
    ApplicationConfig calldata config
) external onlyRole(REGISTRY_ROLE) {
    require(applicationConfigs[domain].primaryValidator != address(0), "Not registered");

    applicationConfigs[domain] = config;

    emit ApplicationConfigUpdated(domain, config.primaryValidator);
}

function getApplicationConfig(bytes32 domain)
    external view returns (ApplicationConfig memory)
{
    return applicationConfigs[domain];
}
```

### 3.2 Nonce Tracking

```solidity
function _validateAndConsumeNonce(bytes32 domain, uint64 nonce) internal {
    uint64 expected = lastNonce[domain] + 1;
    require(nonce == expected, "Invalid nonce");
    lastNonce[domain] = nonce;
}

function getLastNonce(bytes32 domain) external view returns (uint64) {
    return lastNonce[domain];
}

// Nonce is consumed on:
// 1. initializeMessage() - message accepted
// 2. rejectProposal() - message rejected before initialization
// Both are terminal for that nonce
```

### 3.3 Message Expiration

```solidity
function isExpired(bytes32 messageId) public view returns (bool) {
    MessageState storage state = messageStates[messageId];
    if (state.stage == MessageStage.NotInitialized) return false;
    if (state.stage >= MessageStage.Completed) return false;  // Already terminal

    ApplicationConfig storage config = applicationConfigs[state.domain];
    uint256 expirationTime = state.timestamp + config.expirationSeconds;
    return block.timestamp > expirationTime;
}

function expireMessage(bytes32 messageId) external {
    require(isExpired(messageId), "Not expired");
    require(
        messageStates[messageId].stage == MessageStage.Pending ||
        messageStates[messageId].stage == MessageStage.Ready,
        "Cannot expire"
    );

    messageStates[messageId].stage = MessageStage.Expired;
    emit MessageExpired(messageId, block.timestamp);
}
```

## 4. Signature Verification

### 4.1 EIP-712 Domain

```solidity
bytes32 public immutable DOMAIN_SEPARATOR;

constructor(uint256 chainId, address bridgeAddress) {
    DOMAIN_SEPARATOR = keccak256(abi.encode(
        keccak256("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)"),
        keccak256("SyndBridge"),
        keccak256("1"),
        chainId,
        bridgeAddress
    ));
}
```

### 4.2 Message Typehash

```solidity
bytes32 constant MESSAGE_TYPEHASH = keccak256(
    "Message(bytes32 messageId,string messageType,bytes calldata_,bytes32 metadataHash,uint64 nonce,uint64 timestamp,bytes32 domain)"
);

function _computeStructHash(bytes32 messageId) internal view returns (bytes32) {
    MessageState storage state = messageStates[messageId];

    return keccak256(abi.encode(
        MESSAGE_TYPEHASH,
        messageId,
        keccak256(bytes(state.messageType)),
        keccak256(state.calldata_),
        state.metadataHash,    // Hash of canonical metadata JSON
        state.nonce,
        state.timestamp,
        state.domain
    ));
}
```

### 4.3 Signature Recovery

```solidity
function _verifySignature(
    bytes32 messageId,
    bytes calldata signature
) internal view returns (address) {
    bytes32 structHash = _computeStructHash(messageId);

    bytes32 digest = keccak256(abi.encodePacked(
        "\x19\x01",
        DOMAIN_SEPARATOR,
        structHash
    ));

    address signer = ECDSA.recover(digest, signature);
    require(validatorInfo[signer].active, "Not a validator");

    return signer;
}
```

### 4.4 Message ID Computation

Message IDs are deterministic hashes of message content, ensuring the same message always produces the same ID:

```solidity
function computeMessageId(
    string calldata messageType,
    bytes calldata calldata_,
    bytes32 metadataHash,
    uint64 nonce,
    uint64 timestamp,
    bytes32 domain
) public pure returns (bytes32) {
    return keccak256(abi.encode(
        messageType,
        keccak256(calldata_),
        metadataHash,
        nonce,
        timestamp,
        domain
    ));
}
```

**Key properties:**
- Includes `nonce` and `domain` to prevent cross-application collisions
- Includes `timestamp` to differentiate retries with same content
- Uses `keccak256(calldata_)` to handle variable-length data
- Deterministic: same inputs always produce same messageId

**Validation in initializeMessage:**
```solidity
// Caller provides messageId, we verify it matches
bytes32 computed = computeMessageId(messageType, calldata_, metadataHash, nonce, timestamp, domain);
if (messageId != computed) revert InvalidMessageId(messageId, computed);
```

## 5. Message Lifecycle

### 5.1 Initialization

```solidity
function initializeMessage(
    bytes32 messageId,
    string calldata messageType,
    bytes calldata calldata_,
    bytes32 metadataHash,
    string calldata storageRef,
    uint64 nonce,
    uint64 timestamp,
    bytes32 domain
) external payable {
    // 1. Verify caller is Primary Validator for this domain
    ApplicationConfig storage config = applicationConfigs[domain];
    require(config.primaryValidator == msg.sender, "Not primary validator");
    require(config.active, "Application not active");

    // 2. Verify message type is registered and enabled
    MessageTypeConfig storage typeConfig = messageTypes[messageType];
    require(typeConfig.enabled, "Message type not enabled");

    // 3. Verify message not already initialized
    require(messageStates[messageId].stage == MessageStage.NotInitialized, "Already initialized");

    // 4. Validate and consume nonce
    _validateAndConsumeNonce(domain, nonce);

    // 5. Validate timestamp freshness
    require(
        block.timestamp >= timestamp - MAX_CLOCK_DRIFT &&
        block.timestamp <= timestamp + MAX_CLOCK_DRIFT,
        "Timestamp out of range"
    );

    // 6. Handle value (wrap ETH to WETH if sent)
    uint256 value = msg.value;
    if (value > 0) {
        IWETH(WETH).deposit{value: value}();
    }

    // 7. Store message state
    messageStates[messageId] = MessageState({
        stage: MessageStage.Pending,
        messageType: messageType,
        calldata_: calldata_,
        metadataHash: metadataHash,
        storageRef: storageRef,
        value: value,
        nonce: nonce,
        timestamp: timestamp,
        domain: domain,
        primaryValidator: msg.sender,
        signaturesCollected: 0,
        rejectionsCollected: 0
    });

    emit MessageInitialized(messageId, domain, msg.sender, messageType, storageRef);
}

function initializeAndSign(
    bytes32 messageId,
    string calldata messageType,
    bytes calldata calldata_,
    bytes32 metadataHash,
    string calldata storageRef,
    uint64 nonce,
    uint64 timestamp,
    bytes32 domain,
    bytes calldata signature
) external payable {
    initializeMessage(messageId, messageType, calldata_, metadataHash, storageRef, nonce, timestamp, domain);
    signMessage(messageId, signature);
}
```

### 5.2 Signature Collection

```solidity
function signMessage(bytes32 messageId, bytes calldata signature) external {
    MessageState storage state = messageStates[messageId];
    require(state.stage == MessageStage.Pending, "Not pending");
    require(!isExpired(messageId), "Message expired");

    // Verify signature and get signer
    address signer = _verifySignature(messageId, signature);
    require(!hasValidatorSigned[messageId][signer], "Already signed");

    // Record signature
    hasValidatorSigned[messageId][signer] = true;
    state.signaturesCollected++;

    emit SignatureSubmitted(messageId, signer, state.signaturesCollected);

    // Check if threshold reached
    if (state.signaturesCollected >= signatureThreshold) {
        state.stage = MessageStage.Ready;
        emit ThresholdReached(messageId, state.signaturesCollected);
    }
}

function getSignatureCount(bytes32 messageId) external view returns (uint256) {
    return messageStates[messageId].signaturesCollected;
}
```

### 5.3 Rejection Flow

```solidity
// Primary Validator rejects a proposed message BEFORE initialization
// This consumes the nonce to prevent replay
function rejectProposal(
    bytes32 messageId,
    string calldata messageType,
    bytes32 domain,
    uint64 nonce,
    bytes32 reasonHash,
    string calldata reasonRef
) external {
    // Only Primary Validator for this domain can reject proposals
    require(applicationConfigs[domain].primaryValidator == msg.sender, "Not primary");
    require(!proposalRejected[messageId], "Already rejected");
    require(messageStates[messageId].stage == MessageStage.NotInitialized, "Already initialized");

    // Validate and consume nonce (prevents replay of rejected message)
    _validateAndConsumeNonce(domain, nonce);

    // Mark as rejected
    proposalRejected[messageId] = true;

    emit ProposalRejected(messageId, domain, msg.sender, nonce, reasonHash, reasonRef);
}

// Any validator rejects an INITIALIZED message
// This is informational - message can still reach threshold
function rejectMessage(
    bytes32 messageId,
    bytes32 reasonHash,
    string calldata reasonRef
) external {
    require(validatorInfo[msg.sender].active, "Not a validator");
    require(messageStates[messageId].stage == MessageStage.Pending, "Not pending");
    require(!hasValidatorRejected[messageId][msg.sender], "Already rejected");

    hasValidatorRejected[messageId][msg.sender] = true;
    messageStates[messageId].rejectionsCollected++;

    messageRejections[messageId].push(Rejection({
        validator: msg.sender,
        reasonHash: reasonHash,
        reasonRef: reasonRef,
        timestamp: uint64(block.timestamp)
    }));

    emit MessageRejected(messageId, msg.sender, reasonHash, reasonRef);
}

function getRejections(bytes32 messageId) external view returns (Rejection[] memory) {
    return messageRejections[messageId];
}
```

### 5.4 Execution

```solidity
function executeMessage(bytes32 messageId) external nonReentrant {
    MessageState storage state = messageStates[messageId];
    require(state.stage == MessageStage.Ready, "Not ready for execution");
    require(!isExpired(messageId), "Message expired");

    MessageTypeConfig memory config = messageTypes[state.messageType];
    require(config.enabled, "Message type disabled");

    // Stage: Pre-execution modules
    state.stage = MessageStage.PreExecution;
    _runPreModules(messageId, state);

    // Stage: Execute
    state.stage = MessageStage.Executing;

    // If message has value, unwrap WETH before calling
    if (state.value > 0) {
        IWETH(WETH).withdraw(state.value);
    }

    (bool success, bytes memory returnData) = config.target.call{value: state.value}(
        state.calldata_
    );

    if (!success) {
        state.stage = MessageStage.Failed;
        emit MessageFailed(messageId, "Execution reverted", returnData);
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

### 5.5 Expiration

See Section 3.3 for `isExpired()` and `expireMessage()`.

Expiration is a terminal state:
- Nonce was already consumed at initialization
- Message cannot be retried with same nonce
- Application must submit new message with next nonce

## 6. Module System

### 6.1 Module Interface

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

### 6.2 Module Configuration

```solidity
struct ModuleConfig {
    address module;
    bool preExecution;      // Run before execution
    bool postExecution;     // Run after execution
    bool global;            // Apply to all message types
    string[] messageTypes;  // If not global, which message types
}

mapping(address => ModuleConfig) public modules;
mapping(string => address[]) public messageTypeModules;
address[] public globalPreModules;
address[] public globalPostModules;

function addGlobalModule(
    address module,
    bool preExecution,
    bool postExecution
) external onlyRole(ADMIN_ROLE);

function addModuleForTypes(
    address module,
    bool preExecution,
    bool postExecution,
    string[] calldata messageTypes
) external onlyRole(ADMIN_ROLE);

function removeModule(address module) external onlyRole(ADMIN_ROLE);
```

### 6.3 Module Execution

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

function _runPostModules(bytes32 messageId, MessageState storage state) internal {
    // Run global post-modules
    for (uint i = 0; i < globalPostModules.length; i++) {
        (bool pass, string memory reason) = IModule(globalPostModules[i]).check(messageId, false);
        require(pass, reason);
    }

    // Run message-type-specific post-modules
    address[] storage typeModules = messageTypeModules[state.messageType];
    for (uint i = 0; i < typeModules.length; i++) {
        ModuleConfig storage config = modules[typeModules[i]];
        if (config.postExecution) {
            (bool pass, string memory reason) = IModule(typeModules[i]).check(messageId, false);
            require(pass, reason);
        }
    }
}
```

### 6.4 Example Modules

```solidity
// Supply cap check (post-execution)
contract SupplyCapModule is IModule {
    IERC20 public token;
    uint256 public maxSupply;

    function check(bytes32 messageId, bool isPreExecution)
        external view returns (bool pass, string memory reason)
    {
        if (isPreExecution) return (true, "");

        if (token.totalSupply() > maxSupply) {
            return (false, "Supply cap exceeded");
        }
        return (true, "");
    }
}

// Rate limiting (pre-execution)
contract RateLimitModule is IModule {
    mapping(bytes32 => uint256) public messageCount;  // domain -> count
    mapping(bytes32 => uint256) public windowStart;   // domain -> timestamp
    uint256 public windowDuration = 1 hours;
    uint256 public maxPerWindow = 100;

    function check(bytes32 messageId, bool isPreExecution)
        external view returns (bool pass, string memory reason)
    {
        if (!isPreExecution) return (true, "");

        bytes32 domain = IMessageBridge(msg.sender).getMessageState(messageId).domain;

        uint256 currentWindow = block.timestamp / windowDuration;
        if (windowStart[domain] != currentWindow) {
            // New window - would reset count (handled in non-view version)
            return (true, "");
        }

        if (messageCount[domain] >= maxPerWindow) {
            return (false, "Rate limit exceeded");
        }
        return (true, "");
    }
}
```

## 7. Validator Management

### 7.1 TEE Bootstrapping Flow

```
1. ENCLAVE STARTUP
   ├─ Validator starts in TEE (GCP Confidential Space, AWS Nitro, etc.)
   └─ TEE generates attestation token

2. KEY GENERATION
   ├─ Generate secp256k1 signing key INSIDE enclave
   ├─ Key never leaves enclave memory
   └─ Derive Ethereum address from public key

3. ATTESTATION
   ├─ Create attestation binding key to enclave
   ├─ Attestation includes: code hash, key fingerprint
   └─ Sign attestation with TEE platform key

4. REGISTRATION
   ├─ Submit to Bridge: addValidator(address, attestation)
   ├─ Bridge verifies attestation (on-chain or via oracle)
   └─ Validator added to signing set

5. ONGOING OPERATION
   ├─ Validator signs messages with enclave-protected key
   └─ No per-message attestation needed (key already attested)
```

### 7.2 Validator Registration

```solidity
function setPrimaryValidator(
    bytes32 domain,
    address validator,
    bytes calldata attestation
) external onlyRole(VALIDATOR_MANAGER_ROLE) {
    require(_verifyAttestation(validator, attestation), "Invalid attestation");
    require(applicationConfigs[domain].primaryValidator == address(0) ||
            applicationConfigs[domain].primaryValidator == validator, "Primary already set");

    validatorInfo[validator] = ValidatorInfo({
        validator: validator,
        active: true,
        isWitness: false,
        registeredAt: uint64(block.timestamp),
        teeCodeHash: _extractCodeHash(attestation),
        teeVersion: _extractVersion(attestation),
        lastAttestationAt: uint64(block.timestamp)
    });

    applicationConfigs[domain].primaryValidator = validator;

    emit PrimaryValidatorSet(domain, validator, attestation);
}

function addWitnessValidator(
    address validator,
    bytes calldata attestation
) external onlyRole(VALIDATOR_MANAGER_ROLE) {
    require(_verifyAttestation(validator, attestation), "Invalid attestation");
    require(!validatorInfo[validator].active, "Already registered");

    validatorInfo[validator] = ValidatorInfo({
        validator: validator,
        active: true,
        isWitness: true,
        registeredAt: uint64(block.timestamp),
        teeCodeHash: _extractCodeHash(attestation),
        teeVersion: _extractVersion(attestation),
        lastAttestationAt: uint64(block.timestamp)
    });

    witnessValidators.add(validator);

    emit WitnessValidatorAdded(validator, attestation);
}

function removeValidator(address validator) external onlyRole(VALIDATOR_MANAGER_ROLE) {
    require(validatorInfo[validator].active, "Not active");

    validatorInfo[validator].active = false;
    witnessValidators.remove(validator);

    emit ValidatorRemoved(validator);
}
```

### 7.3 Attestation Updates

```solidity
function updateValidatorAttestation(
    address validator,
    bytes calldata attestation,
    string calldata teeVersion
) external onlyRole(VALIDATOR_MANAGER_ROLE) {
    require(validatorInfo[validator].active, "Not active");
    require(_verifyAttestation(validator, attestation), "Invalid attestation");
    require(_isVersionValid(teeVersion), "Version below minimum");

    ValidatorInfo storage info = validatorInfo[validator];
    info.teeCodeHash = _extractCodeHash(attestation);
    info.teeVersion = teeVersion;
    info.lastAttestationAt = uint64(block.timestamp);

    emit ValidatorAttestationUpdated(validator, info.teeCodeHash, teeVersion);
}

function setMinimumTeeVersion(string calldata version) external onlyRole(ADMIN_ROLE) {
    minimumTeeVersion = version;
    emit MinimumTeeVersionUpdated(version);
}

function setSignatureThreshold(uint256 threshold) external onlyRole(VALIDATOR_MANAGER_ROLE) {
    require(threshold > 0, "Threshold must be positive");
    require(threshold <= witnessValidators.length() + 1, "Threshold too high"); // +1 for primary

    uint256 oldThreshold = signatureThreshold;
    signatureThreshold = threshold;

    emit ThresholdUpdated(oldThreshold, threshold);
}
```

## 8. Access Control

```solidity
bytes32 public constant ADMIN_ROLE = keccak256("ADMIN_ROLE");
bytes32 public constant REGISTRY_ROLE = keccak256("REGISTRY_ROLE");
bytes32 public constant VALIDATOR_MANAGER_ROLE = keccak256("VALIDATOR_MANAGER_ROLE");

// Role permissions:
// ADMIN_ROLE:
//   - Grant/revoke all roles
//   - Set minimum TEE version
//   - Emergency pause
//   - Authorize upgrades (with timelock)
//
// REGISTRY_ROLE:
//   - Register/update message types
//   - Register/update applications
//   - Enable/disable message types
//
// VALIDATOR_MANAGER_ROLE:
//   - Add/remove validators
//   - Set signature threshold
//   - Update validator attestations
```

## 9. WETH Handling

The Bridge holds WETH (not native ETH) for consistent accounting and reentrancy safety.

### 9.1 Flow Diagram

```
User/App sends ETH to Bridge (triggers receive())
                    ↓
        Bridge wraps ETH → WETH immediately
                    ↓
        WETH stored in Bridge (internal accounting)
                    ↓
        Primary calls initializeMessage() with value
                    ↓
        Message waits for validator signatures
                    ↓
        executeMessage() called
                    ↓
        Bridge unwraps WETH → ETH
                    ↓
        ETH sent with call to target contract
                    ↓
        Target contract receives native ETH
                    ↓
        If target returns ETH → Bridge receive() auto re-wraps
```

### 9.2 Implementation

```solidity
IWETH public immutable WETH;

receive() external payable {
    // Only wrap ETH if it's not coming from WETH unwrapping
    // This prevents infinite loops during withdraw
    if (msg.sender != address(WETH)) {
        WETH.deposit{value: msg.value}();
        emit NativeTokenWrapped(msg.sender, msg.value);
    }
}

// In executeMessage():
if (state.value > 0) {
    // Verify sufficient WETH balance
    require(WETH.balanceOf(address(this)) >= state.value, "Insufficient WETH");

    // Unwrap WETH to ETH
    WETH.withdraw(state.value);
    emit NativeTokenUnwrapped(state.value, config.target);
}

// Execute with native ETH
(bool success, bytes memory returnData) = config.target.call{value: state.value}(
    state.calldata_
);
```

**Design Rationale:**
- Wrapping ETH immediately prevents reentrancy during initialization
- WETH provides ERC20-like balance tracking
- Failed execution leaves WETH in bridge (no stuck ETH)
- The `msg.sender != WETH` check prevents infinite loops when unwrapping

## 10. Batch Operations

For gas efficiency, the Bridge supports batch operations:

### 10.1 Batch Signature Submission

```solidity
struct SignatureData {
    bytes32 messageId;
    bytes signature;
}

function batchSignMessages(SignatureData[] calldata signatures) external {
    if (signatures.length > MAX_BATCH_SIZE) {
        revert BatchSizeExceeded(signatures.length, MAX_BATCH_SIZE);
    }

    for (uint256 i = 0; i < signatures.length; i++) {
        // Skip if already signed or not pending (don't revert entire batch)
        if (hasValidatorSigned[signatures[i].messageId][msg.sender]) continue;
        if (messageStates[signatures[i].messageId].stage != MessageStage.Pending) continue;

        _signMessageInternal(signatures[i].messageId, signatures[i].signature);
    }
}
```

### 10.2 Batch Execution

```solidity
function batchExecuteMessages(bytes32[] calldata messageIds) external nonReentrant {
    if (messageIds.length > MAX_BATCH_SIZE) {
        revert BatchSizeExceeded(messageIds.length, MAX_BATCH_SIZE);
    }

    for (uint256 i = 0; i < messageIds.length; i++) {
        // Skip if not ready (don't revert entire batch)
        if (messageStates[messageIds[i]].stage != MessageStage.Ready) continue;
        if (isExpired(messageIds[i])) continue;

        _executeMessageInternal(messageIds[i]);
    }
}
```

### 10.3 Batch Query

```solidity
function batchGetMessageStates(bytes32[] calldata messageIds)
    external view returns (MessageState[] memory)
{
    MessageState[] memory states = new MessageState[](messageIds.length);
    for (uint256 i = 0; i < messageIds.length; i++) {
        states[i] = messageStates[messageIds[i]];
    }
    return states;
}
```

## 11. Emergency Functions

### 11.1 Pause/Unpause

```solidity
bool public paused;

modifier whenNotPaused() {
    if (paused) revert ContractPaused();
    _;
}

function pause() external onlyRole(ADMIN_ROLE) {
    paused = true;
    emit Paused(msg.sender);
}

function unpause() external onlyRole(ADMIN_ROLE) {
    paused = false;
    emit Unpaused(msg.sender);
}
```

**Functions affected by pause:**
- `initializeMessage()` / `initializeAndSign()`
- `signMessage()` / `batchSignMessages()`
- `executeMessage()` / `batchExecuteMessages()`

**Functions NOT affected (always available):**
- `rejectProposal()` / `rejectMessage()` - Allow rejections even when paused
- `expireMessage()` - Allow cleanup
- All query functions

### 11.2 Emergency WETH Withdrawal

For recovering stuck WETH (e.g., from failed executions or bugs):

```solidity
function emergencyWithdrawWETH(
    address recipient,
    uint256 amount
) external onlyRole(ADMIN_ROLE) {
    if (recipient == address(0)) revert InvalidAddress("recipient");

    uint256 balance = WETH.balanceOf(address(this));
    if (amount > balance) revert InsufficientWETHBalance(amount, balance);

    WETH.transfer(recipient, amount);
    emit EmergencyWithdrawal(recipient, amount);
}
```

**Safety considerations:**
- Only callable by ADMIN_ROLE
- Consider adding timelock for large withdrawals
- Log all emergency actions for audit trail

### 11.3 Force Expire

For cleaning up stuck messages (e.g., if expiration logic has a bug):

```solidity
function forceExpire(bytes32 messageId) external onlyRole(ADMIN_ROLE) {
    MessageState storage state = messageStates[messageId];

    if (state.stage == MessageStage.NotInitialized) {
        revert MessageNotInitialized(messageId);
    }
    if (state.stage >= MessageStage.Completed) {
        revert MessageAlreadyTerminal(messageId, state.stage);
    }

    state.stage = MessageStage.Expired;
    emit MessageForceExpired(messageId, msg.sender);
}
```

## 12. Upgrade Pattern

Using UUPS (Universal Upgradeable Proxy Standard) with timelock:

```solidity
contract MessageBridge is UUPSUpgradeable, AccessControlUpgradeable {
    uint256 public constant UPGRADE_DELAY = 2 days;
    uint256 public upgradeProposedAt;
    address public proposedImplementation;

    function proposeUpgrade(address newImplementation) external onlyRole(ADMIN_ROLE) {
        proposedImplementation = newImplementation;
        upgradeProposedAt = block.timestamp;
        emit UpgradeProposed(newImplementation, block.timestamp + UPGRADE_DELAY);
    }

    function _authorizeUpgrade(address newImplementation)
        internal
        override
        onlyRole(ADMIN_ROLE)
    {
        require(newImplementation == proposedImplementation, "Not proposed");
        require(
            block.timestamp >= upgradeProposedAt + UPGRADE_DELAY,
            "Upgrade delay not met"
        );
    }
}
```

### 12.1 Storage Layout for Upgrades

UUPS upgrades require careful storage layout management. New storage variables must be appended, never inserted.

```solidity
/**
 * @title MessageBridgeStorage
 * @notice Storage layout for upgradeable MessageBridge
 * @dev CRITICAL: Only append new variables. Never insert or reorder.
 *      Use storage gaps to reserve space for future additions.
 */
abstract contract MessageBridgeStorageV1 {
    // ============ Slot 0-2: Inherited from OpenZeppelin ============
    // AccessControlUpgradeable uses slots 0-1
    // UUPSUpgradeable uses slot 2

    // ============ Slot 3+: Core State ============

    /// @dev Domain separator for EIP-712 (computed once, immutable after init)
    bytes32 internal _domainSeparator;                    // Slot 3

    /// @dev WETH contract address
    address public weth;                                   // Slot 4

    /// @dev Signature threshold (M-of-N)
    uint256 public signatureThreshold;                     // Slot 5

    /// @dev Minimum TEE version string
    string public minimumTeeVersion;                       // Slot 6 (dynamic)

    /// @dev Pause state
    bool public paused;                                    // Slot 7 (packed)

    // ============ Slot 8+: Mappings (don't consume sequential slots) ============

    /// @dev Message states: messageId => MessageState
    mapping(bytes32 => MessageState) internal _messageStates;

    /// @dev Application configs: domain => ApplicationConfig
    mapping(bytes32 => ApplicationConfig) internal _applicationConfigs;

    /// @dev Last nonce per domain: domain => nonce
    mapping(bytes32 => uint64) internal _lastNonce;

    /// @dev Validator info: address => ValidatorInfo
    mapping(address => ValidatorInfo) internal _validatorInfo;

    /// @dev Message type configs: messageType => MessageTypeConfig
    mapping(string => MessageTypeConfig) internal _messageTypes;

    /// @dev Selector to message type: selector => messageType
    mapping(bytes4 => string) internal _selectorToMessageType;

    /// @dev Signature tracking: messageId => validator => signed
    mapping(bytes32 => mapping(address => bool)) internal _hasValidatorSigned;

    /// @dev Rejection tracking: messageId => validator => rejected
    mapping(bytes32 => mapping(address => bool)) internal _hasValidatorRejected;

    /// @dev Rejections: messageId => Rejection[]
    mapping(bytes32 => Rejection[]) internal _messageRejections;

    /// @dev Proposal rejections: messageId => rejected
    mapping(bytes32 => bool) internal _proposalRejected;

    /// @dev Global pre-execution modules
    address[] internal _globalPreModules;

    /// @dev Global post-execution modules
    address[] internal _globalPostModules;

    /// @dev Module configs: module => ModuleConfig
    mapping(address => ModuleConfig) internal _modules;

    /// @dev Message type modules: messageType => modules[]
    mapping(string => address[]) internal _messageTypeModules;

    // ============ Upgrade Management ============

    /// @dev Proposed implementation for upgrade
    address public proposedImplementation;                 // Slot N

    /// @dev Timestamp when upgrade was proposed
    uint256 public upgradeProposedAt;                      // Slot N+1

    // ============ Storage Gap ============
    /// @dev Reserved storage space for future versions
    /// @dev Reduce this gap when adding new storage variables
    uint256[50] private __gap;
}

/**
 * @title MessageBridgeStorageV2
 * @notice Example of how to add storage in V2
 */
abstract contract MessageBridgeStorageV2 is MessageBridgeStorageV1 {
    // New V2 storage variables go here
    // mapping(bytes32 => uint256) internal _newFeatureData;

    // Reduce gap by number of slots used
    uint256[49] private __gap_v2;  // 50 - 1 = 49
}
```

**Storage Layout Rules:**

| Rule | Description |
|------|-------------|
| **Append only** | Never insert variables between existing ones |
| **Use gaps** | Reserve 50 slots for future additions |
| **Document slots** | Comment which slot each variable uses |
| **Avoid inheritance changes** | Don't add new base contracts |
| **Test migrations** | Use Foundry's storage layout checker |

**Foundry Storage Check:**

```bash
# Generate storage layout
forge inspect MessageBridge storage-layout --pretty

# Compare layouts between versions
forge inspect MessageBridgeV1 storage-layout > v1.json
forge inspect MessageBridgeV2 storage-layout > v2.json
diff v1.json v2.json
```

## 13. Attestation Verification

### 13.1 On-Chain Attestation Verifier

For production, attestation verification can be delegated to a specialized verifier contract or oracle.

```solidity
interface IAttestationVerifier {
    struct AttestationData {
        address validator;
        bytes32 codeHash;
        string version;
        uint64 timestamp;
        bytes32 platformId;  // keccak256("gcp-confidential-space"), etc.
    }

    function verify(
        bytes calldata attestation,
        address expectedValidator
    ) external view returns (AttestationData memory);

    function isTrustedPlatform(bytes32 platformId) external view returns (bool);
}

contract MessageBridge {
    IAttestationVerifier public attestationVerifier;

    function _verifyAttestation(
        address validator,
        bytes calldata attestation
    ) internal view returns (bool) {
        if (address(attestationVerifier) == address(0)) {
            // No verifier configured - attestation trusted by admin
            return true;
        }

        AttestationData memory data = attestationVerifier.verify(attestation, validator);

        // Verify platform is trusted
        require(attestationVerifier.isTrustedPlatform(data.platformId), "Untrusted platform");

        // Verify version meets minimum
        require(_isVersionValid(data.version), "Version below minimum");

        // Verify attestation is recent (within 24 hours)
        require(block.timestamp - data.timestamp < 86400, "Attestation expired");

        return true;
    }

    function _extractCodeHash(bytes calldata attestation) internal view returns (bytes32) {
        if (address(attestationVerifier) == address(0)) {
            return bytes32(0);
        }
        return attestationVerifier.verify(attestation, address(0)).codeHash;
    }

    function _extractVersion(bytes calldata attestation) internal view returns (string memory) {
        if (address(attestationVerifier) == address(0)) {
            return "";
        }
        return attestationVerifier.verify(attestation, address(0)).version;
    }

    function setAttestationVerifier(address verifier) external onlyRole(ADMIN_ROLE) {
        attestationVerifier = IAttestationVerifier(verifier);
        emit AttestationVerifierUpdated(verifier);
    }
}
```

### 13.2 GCP Confidential Space Verifier

```solidity
/**
 * @title GCPAttestationVerifier
 * @notice Verifies GCP Confidential Space attestation tokens
 * @dev Uses OIDC token verification with Google's public keys
 */
contract GCPAttestationVerifier is IAttestationVerifier {
    bytes32 public constant GCP_PLATFORM_ID = keccak256("gcp-confidential-space");

    // Google's OIDC public keys (rotated periodically)
    mapping(bytes32 => bytes) public googlePublicKeys;

    // Trusted workload image digests
    mapping(bytes32 => bool) public trustedImageDigests;

    function verify(
        bytes calldata attestation,
        address expectedValidator
    ) external view override returns (AttestationData memory) {
        // 1. Decode JWT token
        (bytes memory header, bytes memory payload, bytes memory signature) =
            _decodeJWT(attestation);

        // 2. Verify signature with Google's public key
        bytes32 keyId = _extractKeyId(header);
        require(googlePublicKeys[keyId].length > 0, "Unknown signing key");
        require(_verifySignature(header, payload, signature, googlePublicKeys[keyId]), "Invalid signature");

        // 3. Extract claims
        JWTClaims memory claims = _decodeClaims(payload);

        // 4. Verify issuer
        require(
            keccak256(bytes(claims.iss)) == keccak256("https://confidentialcomputing.googleapis.com"),
            "Invalid issuer"
        );

        // 5. Verify audience (should be Bridge address)
        require(claims.aud == address(this), "Invalid audience");

        // 6. Verify image digest is trusted
        require(trustedImageDigests[claims.imageDigest], "Untrusted image");

        // 7. Extract validator address from claims
        address validatorFromClaims = _extractValidatorAddress(claims);
        if (expectedValidator != address(0)) {
            require(validatorFromClaims == expectedValidator, "Validator mismatch");
        }

        return AttestationData({
            validator: validatorFromClaims,
            codeHash: claims.imageDigest,
            version: claims.version,
            timestamp: claims.iat,
            platformId: GCP_PLATFORM_ID
        });
    }

    function isTrustedPlatform(bytes32 platformId) external pure override returns (bool) {
        return platformId == GCP_PLATFORM_ID;
    }

    // Admin functions
    function addTrustedImageDigest(bytes32 digest) external onlyOwner {
        trustedImageDigests[digest] = true;
    }

    function updateGooglePublicKey(bytes32 keyId, bytes calldata publicKey) external onlyOwner {
        googlePublicKeys[keyId] = publicKey;
    }
}
```

### 13.3 Simplified Attestation (Development/Testing)

```solidity
/**
 * @title SimpleAttestationVerifier
 * @notice Simplified verifier for development - trusts admin-signed attestations
 * @dev NOT FOR PRODUCTION - use platform-specific verifiers
 */
contract SimpleAttestationVerifier is IAttestationVerifier {
    bytes32 public constant SIMPLE_PLATFORM_ID = keccak256("simple-attestation");

    address public trustedSigner;

    function verify(
        bytes calldata attestation,
        address expectedValidator
    ) external view override returns (AttestationData memory) {
        // Attestation format: abi.encode(validator, codeHash, version, timestamp, signature)
        (
            address validator,
            bytes32 codeHash,
            string memory version,
            uint64 timestamp,
            bytes memory signature
        ) = abi.decode(attestation, (address, bytes32, string, uint64, bytes));

        // Verify signature from trusted signer
        bytes32 digest = keccak256(abi.encode(validator, codeHash, version, timestamp));
        address signer = ECDSA.recover(digest, signature);
        require(signer == trustedSigner, "Invalid attestation signature");

        if (expectedValidator != address(0)) {
            require(validator == expectedValidator, "Validator mismatch");
        }

        return AttestationData({
            validator: validator,
            codeHash: codeHash,
            version: version,
            timestamp: timestamp,
            platformId: SIMPLE_PLATFORM_ID
        });
    }

    function isTrustedPlatform(bytes32 platformId) external pure override returns (bool) {
        return platformId == SIMPLE_PLATFORM_ID;
    }
}
```

## 14. Events

```solidity
// Message lifecycle
event MessageInitialized(
    bytes32 indexed messageId,
    bytes32 indexed domain,
    address primaryValidator,
    string messageType,
    string storageRef
);
event SignatureSubmitted(bytes32 indexed messageId, address indexed validator, uint256 count);
event MessageRejected(bytes32 indexed messageId, address indexed validator, bytes32 reasonHash, string reasonRef);
event ProposalRejected(
    bytes32 indexed messageId,
    bytes32 indexed domain,
    address indexed primaryValidator,
    uint64 nonce,
    bytes32 reasonHash,
    string reasonRef
);
event ThresholdReached(bytes32 indexed messageId, uint256 signatures);
event MessageExecuted(bytes32 indexed messageId, string messageType, address target);
event MessageFailed(bytes32 indexed messageId, string reason, bytes data);
event MessageExpired(bytes32 indexed messageId, uint256 expiredAt);

// Validator management
event PrimaryValidatorSet(bytes32 indexed domain, address indexed validator, bytes attestation);
event WitnessValidatorAdded(address indexed validator, bytes attestation);
event ValidatorRemoved(address indexed validator);
event ValidatorAttestationUpdated(address indexed validator, bytes32 teeCodeHash, string teeVersion);
event ThresholdUpdated(uint256 oldThreshold, uint256 newThreshold);
event MinimumTeeVersionUpdated(string version);

// Application management
event ApplicationRegistered(bytes32 indexed domain, address primaryValidator, uint64 expirationSeconds);
event ApplicationConfigUpdated(bytes32 indexed domain, address primaryValidator);

// Modules
event ModuleAdded(address indexed module, bool preExecution, bool postExecution);
event ModuleRemoved(address indexed module);

// Registry
event MessageTypeRegistered(string indexed messageType, address target, bytes32 schemaHash);
event MessageTypeUpdated(string indexed messageType, bytes32 oldSchemaHash, bytes32 newSchemaHash);
event MessageTypeEnabled(string indexed messageType, bool enabled);

// WETH
event NativeTokenWrapped(address indexed sender, uint256 amount);
event NativeTokenUnwrapped(uint256 amount, address indexed target);

// Upgrades
event UpgradeProposed(address indexed implementation, uint256 effectiveAt);

// Emergency
event Paused(address indexed by);
event Unpaused(address indexed by);
event EmergencyWithdrawal(address indexed recipient, uint256 amount);
event MessageForceExpired(bytes32 indexed messageId, address indexed by);
```

## 15. Full Interfaces

### 15.1 IMessageBridge

```solidity
interface IMessageBridge {
    // Message Initialization
    function initializeMessage(
        bytes32 messageId,
        string calldata messageType,
        bytes calldata calldata_,
        bytes32 metadataHash,
        string calldata storageRef,
        uint64 nonce,
        uint64 timestamp,
        bytes32 domain
    ) external payable;

    function initializeAndSign(
        bytes32 messageId,
        string calldata messageType,
        bytes calldata calldata_,
        bytes32 metadataHash,
        string calldata storageRef,
        uint64 nonce,
        uint64 timestamp,
        bytes32 domain,
        bytes calldata signature
    ) external payable;

    // Signature Submission
    function signMessage(bytes32 messageId, bytes calldata signature) external;
    function rejectMessage(bytes32 messageId, bytes32 reasonHash, string calldata reasonRef) external;
    function rejectProposal(
        bytes32 messageId,
        string calldata messageType,
        bytes32 domain,
        uint64 nonce,
        bytes32 reasonHash,
        string calldata reasonRef
    ) external;

    // Execution
    function executeMessage(bytes32 messageId) external;
    function isExpired(bytes32 messageId) external view returns (bool);
    function expireMessage(bytes32 messageId) external;

    // Validator Management
    function setPrimaryValidator(bytes32 domain, address validator, bytes calldata attestation) external;
    function addWitnessValidator(address validator, bytes calldata attestation) external;
    function removeValidator(address validator) external;
    function setSignatureThreshold(uint256 threshold) external;

    // Queries
    function getMessageState(bytes32 messageId) external view returns (MessageState memory);
    function getSignatureCount(bytes32 messageId) external view returns (uint256);
    function getRejectionCount(bytes32 messageId) external view returns (uint256);
    function getRejections(bytes32 messageId) external view returns (Rejection[] memory);
    function hasValidatorSigned(bytes32 messageId, address validator) external view returns (bool);
    function hasValidatorRejected(bytes32 messageId, address validator) external view returns (bool);
    function isMessageExecuted(bytes32 messageId) external view returns (bool);
    function getPrimaryValidator(bytes32 domain) external view returns (address);
    function getWitnessValidators() external view returns (address[] memory);
    function getSignatureThreshold() external view returns (uint256);
    function getLastNonce(bytes32 domain) external view returns (uint64);
    function getApplicationConfig(bytes32 domain) external view returns (ApplicationConfig memory);
}
```

### 15.2 IMessageTypeRegistry

```solidity
interface IMessageTypeRegistry {
    function registerMessageType(
        string calldata messageType,
        address target,
        bytes32 schemaHash,
        string calldata schemaUri
    ) external;

    function updateSchema(
        string calldata messageType,
        bytes32 newSchemaHash,
        string calldata newSchemaUri
    ) external;

    function setEnabled(string calldata messageType, bool enabled) external;
    function setTarget(string calldata messageType, address newTarget) external;

    function isRegistered(string calldata messageType) external view returns (bool);
    function isEnabled(string calldata messageType) external view returns (bool);
    function getConfig(string calldata messageType) external view returns (MessageTypeConfig memory);
}
```

## 16. Implementation Checklist

### Core Contract
- [ ] MessageBridge.sol with UUPS upgradeable
- [ ] IMessageBridge interface
- [ ] IMessageTypeRegistry interface
- [ ] IModule interface

### Data Structures
- [ ] MessageState struct with all fields
- [ ] ApplicationConfig struct
- [ ] ValidatorInfo struct
- [ ] Rejection struct
- [ ] MessageTypeConfig struct
- [ ] MessageStage enum with Expired

### Application Management
- [ ] registerApplication()
- [ ] updateApplicationConfig()
- [ ] getApplicationConfig()
- [ ] Per-domain nonce tracking

### Message Lifecycle
- [ ] initializeMessage() with nonce consumption
- [ ] initializeAndSign()
- [ ] signMessage() with threshold check
- [ ] rejectProposal() with nonce consumption
- [ ] rejectMessage() with rejection storage
- [ ] executeMessage() with modules
- [ ] isExpired() / expireMessage()

### Signature Verification
- [ ] EIP-712 domain separator
- [ ] Message typehash with metadataHash
- [ ] Signature recovery and validation

### Module System
- [ ] Global pre/post modules
- [ ] Per-message-type modules
- [ ] Module configuration storage
- [ ] Example: SupplyCapModule
- [ ] Example: RateLimitModule

### Validator Management
- [ ] setPrimaryValidator()
- [ ] addWitnessValidator()
- [ ] removeValidator()
- [ ] updateValidatorAttestation()
- [ ] setSignatureThreshold()
- [ ] setMinimumTeeVersion()

### Access Control
- [ ] ADMIN_ROLE
- [ ] REGISTRY_ROLE
- [ ] VALIDATOR_MANAGER_ROLE
- [ ] Role-based function guards

### WETH Handling
- [ ] WETH immutable reference
- [ ] receive() with sender check
- [ ] Wrap on deposit
- [ ] Unwrap before execution

### Batch Operations
- [ ] batchSignMessages()
- [ ] batchExecuteMessages()
- [ ] batchGetMessageStates()
- [ ] MAX_BATCH_SIZE enforcement

### Emergency Functions
- [ ] pause() / unpause()
- [ ] whenNotPaused modifier
- [ ] emergencyWithdrawWETH()
- [ ] forceExpire()

### Storage Layout (Section 12.1)
- [ ] MessageBridgeStorageV1 abstract contract
- [ ] Documented slot assignments
- [ ] 50-slot storage gap
- [ ] Foundry storage-layout verification script

### Attestation Verification (Section 13)
- [ ] IAttestationVerifier interface
- [ ] GCPAttestationVerifier for production
- [ ] SimpleAttestationVerifier for development
- [ ] Trusted image digest management
- [ ] setAttestationVerifier() admin function

### Events
- [ ] All 25+ events with indexed parameters
- [ ] AttestationVerifierUpdated event

### Testing
- [ ] Unit tests for each function
- [ ] Integration tests for full flows
- [ ] Fuzz tests for edge cases
- [ ] Gas optimization tests
