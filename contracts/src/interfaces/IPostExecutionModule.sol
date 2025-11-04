// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {ExecutionContext} from "../types/DataTypes.sol";

/**
 * @title IPostExecutionModule
 * @notice Interface for PostExecution processing modules
 * @dev PostExecution modules process results after core execution and can enforce
 *      post-execution invariants. Modules can revert to block message completion.
 */
interface IPostExecutionModule {
    /// @notice Custom errors for post-execution failures
    error PostExecutionFailed(bytes32 messageId, uint256 errorCode);

    /**
     * @notice Process results after message execution
     * @dev Called by Bridge during PostExecution stage
     *      Can revert to block message completion
     * @param messageId Unique message identifier
     * @param success Whether core execution succeeded
     * @param result Execution result data
     * @param context Execution context
     */
    function afterExecution(bytes32 messageId, bool success, bytes calldata result, ExecutionContext calldata context)
        external;
}
