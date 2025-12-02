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
     * @dev Only callable by addresses with SEQUENCER_ROLE
     * @param messageId Unique identifier for the message
     * @param targetAddress Address that will receive the message call
     * @param payload Encoded function call data
     * @param sequencerSignature Signature from the trusted sequencer
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
     * @dev Validates pre-modules, executes the call, validates post-modules, and marks as completed
     * @param messageId Unique identifier of the message to execute
     */
    function handleMessage(bytes32 messageId) external;

    /**
     * @notice Initializes and immediately executes a message in a single transaction
     * @dev Combines initialization, validator signature collection, and execution
     * @param messageId Unique identifier for the message
     * @param targetAddress Address that will receive the message call
     * @param payload Encoded function call data
     * @param sequencerSignature Signature from the trusted sequencer
     * @param validatorSignatures Array of signatures from authorized validators
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
     * @return bool True if the message reached the Completed stage
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
     * @return bool True if the message has been initialized
     */
    function isMessageInitialized(bytes32 messageId) external view returns (bool);
}
