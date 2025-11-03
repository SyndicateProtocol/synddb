// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {ExecutionContext} from "../types/DataTypes.sol";

/**
 * @title IPreExecutionModule
 * @notice Interface for PreExecution validation modules
 * @dev PreExecution modules validate messages before core execution.
 *      Any module returning false will reject the message.
 */
interface IPreExecutionModule {
    /**
     * @notice Validate a message before execution
     * @dev Called by Bridge during PreExecution stage
     * @param messageId Unique message identifier from DA layer
     * @param payload Message data containing type and parameters
     * @param context Execution context with validator signatures
     * @return shouldExecute True if validation passes, false to reject
     * @return reason If false, explanation for rejection
     */
    function beforeExecution(
        bytes32 messageId,
        bytes calldata payload,
        ExecutionContext calldata context
    ) external returns (bool shouldExecute, string memory reason);
}
