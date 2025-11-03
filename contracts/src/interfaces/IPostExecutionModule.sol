// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {ExecutionContext} from "../types/DataTypes.sol";

/**
 * @title IPostExecutionModule
 * @notice Interface for PostExecution processing modules
 * @dev PostExecution modules process results after core execution.
 *      Module failures do not block message completion.
 */
interface IPostExecutionModule {
    /**
     * @notice Process results after message execution
     * @dev Called by Bridge during PostExecution stage
     * @param messageId Unique message identifier from DA layer
     * @param success Whether core execution succeeded
     * @param result Execution result data
     * @param context Execution context
     */
    function afterExecution(
        bytes32 messageId,
        bool success,
        bytes calldata result,
        ExecutionContext calldata context
    ) external;
}
