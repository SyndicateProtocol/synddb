// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {SequencerSignature} from "src/types/DataTypes.sol";

/**
 * @title IBridge
 * @notice Interface for the cross-chain message bridge
 * @dev Manages message lifecycle from initialization through execution and validation
 */
interface IBridge {
    /**
     * @notice Initializes a new cross-chain message
     * @dev Only callable by addresses with MESSAGE_INITIALIZER_ROLE (sequencers, relayers, etc.).
     *      This role restriction prevents frontrunning and message censorship.
     * @param messageId Unique identifier for the message (bytes32). Recommended schemes for idempotency:
     *        - Sequential nonces: `keccak256(abi.encodePacked(chainId, nonce))`
     *        - UUIDs: Must be hashed to bytes32, e.g., `keccak256(abi.encodePacked(uuidString))`
     *        - Hash of message data: `keccak256(abi.encodePacked(sourceChain, sender, nonce, payload))`
     * @param targetAddress Address that will receive the message call
     * @param payload Encoded function call data (e.g., `abi.encodeWithSignature("transfer(address,uint256)", recipient, amount)`)
     * @param sequencerSignature Signature from the trusted TEE sequencer.
     * @param nativeTokenAmount Amount of native token to transfer with the call
     */
    function initializeMessage(
        bytes32 messageId,
        address targetAddress,
        bytes calldata payload,
        SequencerSignature calldata sequencerSignature,
        uint256 nativeTokenAmount
    ) external;

    /**
     * @notice Executes a previously initialized message
     * @dev Execution follows these steps:
     *      1. Validates message is in PreExecution stage (see ProcessingStage enum in DataTypes.sol)
     *      2. Runs all pre-execution validation modules (ModuleCheck)
     *      3. Unwraps native tokens if nativeTokenAmount > 0
     *      4. Executes the call to targetAddress with payload
     *      5. Re-wraps any returned native tokens
     *      6. Runs all post-execution validation modules (ModuleCheck)
     *      7. Marks message as Completed
     * @param messageId Unique identifier of the message to execute
     */
    function handleMessage(bytes32 messageId) external;

    /**
     * @notice Initializes and immediately executes a message in a single transaction
     * @dev Combines initialization, validator signature collection, and execution.
     *      This is an optimization for fast-path execution when all signatures are available upfront.
     * @param messageId Unique identifier for the message
     * @param targetAddress Address that will receive the message call
     * @param payload Encoded function call data
     * @param sequencerSignature Signature from the trusted TEE sequencer.
     * @param validatorSignatures Array of signatures from authorized validators. Can be empty if running in sequencer-only
     *        mode or if no validator signature threshold module is configured.
     * @param nativeTokenAmount Amount of native token to transfer with the call
     */
    function initializeAndHandleMessage(
        bytes32 messageId,
        address targetAddress,
        bytes calldata payload,
        SequencerSignature calldata sequencerSignature,
        bytes[] calldata validatorSignatures,
        uint256 nativeTokenAmount
    ) external;

    /**
     * @notice Checks if a message has been successfully completed
     * @param messageId Unique identifier of the message
     * @return bool True if the message reached the Completed stage (see ProcessingStage.Completed in DataTypes.sol)
     */
    function isMessageCompleted(bytes32 messageId) external view returns (bool);

    /**
     * @notice Checks if a message has been rejected
     * @param messageId Unique identifier of the message
     * @return bool True if the message reached the Rejected stage
     */
    function isMessageRejected(bytes32 messageId) external view returns (bool);

    /**
     * @notice Checks if a message has finished processing (completed or rejected)
     * @param messageId Unique identifier of the message
     * @return bool True if the message is in a terminal state
     */
    function isMessageHandled(bytes32 messageId) external view returns (bool);

    /**
     * @notice Checks if a message has been initialized
     * @param messageId Unique identifier of the message
     * @return bool True if the message has been initialized (i.e., NOT in ProcessingStage.NotStarted, see DataTypes.sol)
     */
    function isMessageInitialized(bytes32 messageId) external view returns (bool);
}
