// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {ExecutionContext} from "../types/DataTypes.sol";

/**
 * @title IBridge
 * @notice Interface for the modular cross-chain message bridge
 * @dev The Bridge processes messages from SyndDB validators in four stages:
 *      1. Initialization - Create message state
 *      2. PreExecution - Run validation modules
 *      3. Core Execution - Execute the message
 *      4. PostExecution - Run post-processing modules
 */
interface IBridge {
    /**
     * @notice Processing stages for messages
     */
    enum ProcessingStage {
        NotStarted,
        PreExecution,
        Executing,
        PostExecution,
        Completed,
        Rejected
    }

    /**
     * @notice Message state tracking
     */
    struct MessageState {
        bytes32 messageId;
        uint256 preModuleCheckpoint;
        uint256 postModuleCheckpoint;
        ProcessingStage stage;
        bytes payload;
        bool executionSuccess;
        bytes executionResult;
    }

    // ========== Stage Execution Functions ==========

    /**
     * @notice Initialize a new message for processing
     * @dev Creates message state and stores payload
     * @param messageId Unique identifier from DA layer
     * @param payload Message data containing type and parameters
     * @param context Execution context with validator signatures
     */
    function initializeMessage(
        bytes32 messageId,
        bytes calldata payload,
        ExecutionContext calldata context
    ) external;

    /**
     * @notice Execute PreExecution modules for validation
     * @dev Runs modules from current checkpoint. Any module can reject the message.
     * @param messageId The message to validate
     */
    function executePreModules(bytes32 messageId) external;

    /**
     * @notice Execute the core message logic
     * @dev Decodes message type and routes to appropriate handler
     * @param messageId The message to execute
     */
    function executeMessage(bytes32 messageId) external;

    /**
     * @notice Execute PostExecution modules for post-processing
     * @dev Runs modules from current checkpoint. Module failures do not block completion.
     * @param messageId The message to process
     */
    function executePostModules(bytes32 messageId) external;

    /**
     * @notice Convenience function to process message through all stages
     * @dev Executes initialize, pre, core, and post in sequence
     *      OPTIONAL helper - use only if gas limits allow
     * @param messageId Unique identifier from DA layer
     * @param payload Message data
     * @param context Execution context
     */
    function processMessageComplete(
        bytes32 messageId,
        bytes calldata payload,
        ExecutionContext calldata context
    ) external;

    // ========== Module Management Functions ==========

    /**
     * @notice Add a PreExecution module to the registry
     * @param module Address of the module contract
     */
    function addPreExecutionModule(address module) external;

    /**
     * @notice Add a PostExecution module to the registry
     * @param module Address of the module contract
     */
    function addPostExecutionModule(address module) external;

    /**
     * @notice Remove a PreExecution module from the registry
     * @param module Address of the module to remove
     */
    function removePreExecutionModule(address module) external;

    /**
     * @notice Remove a PostExecution module from the registry
     * @param module Address of the module to remove
     */
    function removePostExecutionModule(address module) external;

    /**
     * @notice Get all registered PreExecution modules
     * @return Array of module addresses
     */
    function getPreExecutionModules() external view returns (address[] memory);

    /**
     * @notice Get all registered PostExecution modules
     * @return Array of module addresses
     */
    function getPostExecutionModules() external view returns (address[] memory);

    // ========== View Functions ==========

    /**
     * @notice Check if a message has been processed
     * @param messageId The message to check
     * @return True if message has been processed
     */
    function processedMessages(bytes32 messageId) external view returns (bool);

    /**
     * @notice Get the state of a message
     * @param messageId The message to query
     * @return MessageState struct containing message details
     */
    function messageStates(bytes32 messageId) external view returns (MessageState memory);
}
