// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

/**
 * @title DataTypes
 * @notice Core data structures and enums used throughout the Message Passing Bridge system
 */

// ============================================================
// LEGACY TYPES (kept for backwards compatibility with Bridge.sol)
// ============================================================

/**
 * @notice Legacy lifecycle stages (used by Bridge.sol)
 * @dev New contracts should use MessageStage instead
 */
enum ProcessingStage {
    NotStarted, // Message has not been initialized
    PreExecution, // Message initialized, awaiting pre-execution validation
    Executing, // Message is currently being executed
    PostExecution, // Execution complete, awaiting post-execution validation
    Completed, // Message successfully processed
    Rejected // Message rejected during validation
}

/**
 * @notice Legacy message state (used by Bridge.sol)
 * @dev New contracts should use MessageStateV2 instead
 */
struct MessageState {
    bytes32 messageId; // Unique identifier for the message
    address targetAddress; // Address that will receive the message call
    ProcessingStage stage; // Current processing stage
    bytes payload; // Encoded function call data
    uint256 createdAt; // Block timestamp when message was created
    uint256 nativeTokenAmount; // Amount of native token to transfer with call
}

/**
 * @notice Legacy sequencer signature (used by Bridge.sol)
 * @dev New contracts use EIP-712 signatures instead
 */
struct SequencerSignature {
    bytes signature; // ECDSA signature from the sequencer
    uint256 submittedAt; // Block timestamp when signature was submitted
}

// ============================================================
// NEW TYPES (for MessageBridge.sol - Multi-Validator Architecture)
// ============================================================

/**
 * @notice Message lifecycle stages for multi-validator architecture
 * @dev Terminal states (Completed, Failed, Expired) consume the nonce
 */
enum MessageStage {
    NotInitialized, // Message doesn't exist
    Pending, // Initialized, collecting signatures
    Ready, // Threshold met, awaiting execution
    PreExecution, // Running pre-execution modules
    Executing, // Calling target contract
    PostExecution, // Running post-execution modules
    Completed, // Successfully executed (terminal, nonce consumed)
    Failed, // Execution failed (terminal, nonce consumed)
    Expired // Threshold not reached in time (terminal, nonce consumed)
}

/**
 * @notice Complete message state for multi-validator architecture
 * @dev Contains all information needed to validate, sign, and execute a message
 */
struct MessageStateV2 {
    MessageStage stage; // Current lifecycle stage
    string messageType; // ABI signature (e.g., "mint(address,uint256)")
    bytes calldata_; // ABI-encoded function parameters
    bytes32 metadataHash; // keccak256 of canonical metadata JSON
    string storageRef; // URI(s) to full metadata (ar://...|ipfs://...)
    uint256 value; // Native token amount (stored as WETH)
    uint64 nonce; // Per-domain sequential nonce
    uint64 timestamp; // Message creation timestamp
    bytes32 domain; // Application identifier
    address primaryValidator; // Who initialized this message
    uint256 signaturesCollected; // Count of valid signatures
    uint256 rejectionsCollected; // Count of rejections (informational)
}

/**
 * @notice Application configuration for a domain
 * @dev Each application registers with a domain and Primary Validator
 */
struct ApplicationConfig {
    address primaryValidator; // Who can initialize messages for this domain
    uint64 expirationSeconds; // Message expiration (default: 86400 = 24h)
    bool requireWitnessSignatures; // If true, threshold > 1 required
    bool active; // Whether application is currently active
}

/**
 * @notice Validator registration information
 * @dev Tracks both Primary (domain-specific) and Witness (global) validators
 */
struct ValidatorInfo {
    address validator; // Validator address
    bool active; // Whether validator is currently active
    bool isWitness; // false = Primary (domain-specific), true = Witness (global)
    uint64 registeredAt; // When validator was registered
    bytes32 teeCodeHash; // Hash of validator code running in TEE
    string teeVersion; // Semantic version (e.g., "1.2.3")
    uint64 lastAttestationAt; // When attestation was last verified
}

/**
 * @notice Message type configuration
 * @dev Defines schema and target for each registered message type
 */
struct MessageTypeConfig {
    bytes4 selector; // bytes4(keccak256(messageType))
    address target; // Contract to call when executing
    bytes32 schemaHash; // keccak256 of metadata JSON Schema
    string schemaUri; // Where to fetch full schema (IPFS, Arweave)
    bool enabled; // Whether currently active
    uint64 updatedAt; // Last update timestamp
}

/**
 * @notice Rejection record for a message
 * @dev Messages can have rejections AND still execute if threshold is met
 */
struct Rejection {
    address validator; // Who rejected
    bytes32 reasonHash; // keccak256 of rejection reason JSON
    string reasonRef; // Storage reference to full reason
    uint64 timestamp; // When rejected
}
