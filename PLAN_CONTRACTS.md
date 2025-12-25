# Bridge Contract Implementation Plan

> Implementation details for the Message Passing Bridge smart contracts.
> See `SPEC_MESSAGE_BRIDGE.md` for the specification.

## Overview

This document contains implementation details, code patterns, and Solidity examples for the Bridge contract system.

## 1. Contract Architecture

```
contracts/
├── MessageBridge.sol          # Main bridge contract (UUPS upgradeable)
├── interfaces/
│   ├── IMessageBridge.sol     # Core interface
│   └── IModule.sol            # Module interface
├── modules/
│   ├── RateLimitModule.sol    # Global rate limiting
│   ├── AmountThresholdModule.sol
│   ├── AllowlistModule.sol
│   ├── TimelockModule.sol
│   └── SupplyCapModule.sol    # Post-execution supply check
└── libraries/
    └── SignatureVerifier.sol  # EIP-712 signature verification
```

## 2. Core Implementation

### 2.1 Message State Machine

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
    bytes32 domain;
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

### 2.2 Signature Verification (EIP-712)

```solidity
bytes32 constant MESSAGE_TYPEHASH = keccak256(
    "Message(bytes32 messageId,string messageType,bytes calldata_,bytes32 metadataHash,uint64 nonce,uint64 timestamp,bytes32 domain)"
);

function _verifySignature(
    bytes32 messageId,
    bytes calldata signature
) internal view returns (address) {
    bytes32 structHash = keccak256(abi.encode(
        MESSAGE_TYPEHASH,
        messageId,
        messageStates[messageId].messageType,
        keccak256(messageStates[messageId].calldata_),
        messageStates[messageId].metadataHash,
        messageStates[messageId].nonce,
        messageStates[messageId].timestamp,
        messageStates[messageId].domain
    ));

    bytes32 digest = keccak256(abi.encodePacked(
        "\x19\x01",
        DOMAIN_SEPARATOR,
        structHash
    ));

    address signer = ECDSA.recover(digest, signature);
    require(isValidator[signer], "Not a validator");

    return signer;
}
```

### 2.3 Execution Logic

```solidity
function executeMessage(bytes32 messageId) external nonReentrant {
    MessageState storage state = messageStates[messageId];

    require(state.stage == MessageStage.Ready, "Not ready for execution");

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

## 3. Module System

### 3.1 Module Interface

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

### 3.2 Module Configuration

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

function addGlobalModule(
    address module,
    bool preExecution,
    bool postExecution
) external;

function addModuleForTypes(
    address module,
    bool preExecution,
    bool postExecution,
    string[] calldata messageTypes
) external;
```

### 3.3 Module Execution

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

### 3.4 Example Modules

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
    // Limit messages per time window per application
    mapping(bytes32 => uint256) public messageCount;
    mapping(bytes32 => uint256) public windowStart;
    uint256 public windowDuration = 1 hours;
    uint256 public maxPerWindow = 100;

    function check(bytes32 messageId, bool isPreExecution)
        external view returns (bool pass, string memory reason)
    {
        // Implementation...
    }
}
```

## 4. TEE Bootstrapping

### 4.1 Flow

```
1. ENCLAVE STARTUP
   ├─ Validator starts in TEE (GCP Confidential Space, etc.)
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

### 4.2 Validator Info Tracking

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
string public minimumTeeVersion;

function updateValidatorAttestation(
    address validator,
    bytes calldata attestation,
    string calldata teeVersion
) external {
    require(_verifyAttestation(validator, attestation), "Invalid attestation");

    ValidatorInfo storage info = validatorInfo[validator];
    info.teeCodeHash = _extractCodeHash(attestation);
    info.teeVersion = teeVersion;
    info.lastAttestationAt = uint64(block.timestamp);

    emit ValidatorAttestationUpdated(validator, info.teeCodeHash, teeVersion);
}
```

## 5. Upgrade Pattern

Using UUPS (Universal Upgradeable Proxy Standard):

```solidity
contract MessageBridge is UUPSUpgradeable, AccessControlUpgradeable {
    function _authorizeUpgrade(address newImplementation)
        internal
        override
        onlyRole(ADMIN_ROLE)
    {
        require(
            block.timestamp >= upgradeProposedAt + UPGRADE_DELAY,
            "Upgrade delay not met"
        );
    }
}
```

## 6. WETH Handling

Bridge only holds WETH, not native ETH:

```solidity
function executeMessage(bytes32 messageId) external nonReentrant {
    MessageState storage state = messageStates[messageId];

    // If message has value, unwrap WETH before calling
    if (state.value > 0) {
        IWETH(WETH).withdraw(state.value);
    }

    // Execute with native ETH
    (bool success, bytes memory returnData) = config.target.call{value: state.value}(
        state.calldata_
    );

    // ...
}

// Bridge receives ETH back, wrap to WETH
receive() external payable {
    IWETH(WETH).deposit{value: msg.value}();
}
```

## 7. Access Control

```solidity
bytes32 public constant ADMIN_ROLE = keccak256("ADMIN_ROLE");
bytes32 public constant REGISTRY_ROLE = keccak256("REGISTRY_ROLE");
bytes32 public constant VALIDATOR_MANAGER_ROLE = keccak256("VALIDATOR_MANAGER_ROLE");

// Permissions:
// - ADMIN_ROLE: Can grant/revoke all roles
// - REGISTRY_ROLE: Can register/update message types
// - VALIDATOR_MANAGER_ROLE: Can add/remove validators, set threshold
```

## 8. Events

```solidity
// Message lifecycle
event MessageInitialized(bytes32 indexed messageId, bytes32 indexed domain, address primaryValidator, string messageType, string storageRef);
event SignatureSubmitted(bytes32 indexed messageId, address indexed validator, uint256 count);
event MessageRejected(bytes32 indexed messageId, address indexed validator, bytes32 reasonHash, string reasonRef);
event ProposalRejected(bytes32 indexed messageId, bytes32 indexed domain, address indexed primaryValidator, uint64 nonce, bytes32 reasonHash, string reasonRef);
event ThresholdReached(bytes32 indexed messageId, uint256 signatures);
event MessageExecuted(bytes32 indexed messageId, string messageType, address target);
event MessageFailed(bytes32 indexed messageId, string reason, bytes data);

// Validator management
event PrimaryValidatorSet(bytes32 indexed domain, address indexed validator, bytes attestation);
event WitnessValidatorAdded(address indexed validator, bytes attestation);
event ValidatorRemoved(address indexed validator);
event ThresholdUpdated(uint256 oldThreshold, uint256 newThreshold);

// Modules
event ModuleAdded(address indexed module, bool preExecution, bool postExecution);
event ModuleRemoved(address indexed module);

// Registry
event MessageTypeRegistered(string indexed messageType, address target, bytes32 schemaHash);
event MessageTypeUpdated(string indexed messageType, bytes32 oldSchemaHash, bytes32 newSchemaHash);
event MessageTypeEnabled(string indexed messageType, bool enabled);
```

## 9. Implementation Checklist

- [ ] Core MessageBridge contract
- [ ] IMessageBridge interface
- [ ] IModule interface
- [ ] EIP-712 signature verification
- [ ] Message state machine
- [ ] Nonce tracking
- [ ] Expiration logic
- [ ] Module system (pre/post execution)
- [ ] Access control (roles)
- [ ] UUPS upgrade pattern
- [ ] WETH handling
- [ ] TEE attestation verification
- [ ] Example modules (RateLimit, SupplyCap, Allowlist)
- [ ] Comprehensive test suite
