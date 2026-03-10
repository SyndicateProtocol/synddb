// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

/**
 * @title DataTypes
 * @notice Core data structures and enums used throughout the SyndDB bridge system
 */

/**
 * @notice Types of TEE signing keys managed by the bridge
 */
enum KeyType {
    Sequencer, // Keys used by sequencers to sign messages
    Validator // Keys used by validators to co-sign messages
}

/**
 * @notice Represents the lifecycle stages of a cross-chain message
 * @dev Used to track message processing state and prevent re-entrancy
 */
enum ProcessingStage {
    NotStarted, // Message has not been initialized
    PreExecution, // Message initialized, awaiting pre-execution validation (ModuleCheck.check() is called at this stage)
    Executing, // Message is currently being executed
    PostExecution, // Execution complete, awaiting post-execution validation (ModuleCheck.check() is called at this stage)
    Completed, // Message successfully processed
    Rejected // Message rejected during validation
}

/**
 * @notice Stores the complete state of a cross-chain message
 * @dev Contains all information needed to process and validate a message
 */
struct MessageState {
    bytes32 messageId; // Unique identifier for the message
    address targetAddress; // Address that will receive the message call
    ProcessingStage stage; // Current processing stage
    bytes32 payloadHash; // Hash of encoded function call data (storage optimization)
    uint256 createdAt; // Block timestamp when message was created
    uint256 deadline; // Timestamp after which message cannot be executed (0 = no deadline)
    uint256 nativeTokenAmount; // Amount of native token to transfer with call
}

/**
 * @notice Contains a sequencer's cryptographic signature with timestamp
 * @dev Used to verify message authenticity from the trusted sequencer
 */
struct SequencerSignature {
    bytes signature; // ECDSA signature from the sequencer
    uint256 submittedAt; // Block timestamp when signature was submitted
}
