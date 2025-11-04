// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {ExecutionContext} from "../types/DataTypes.sol";

/**
 * @title IPreExecutionModule
 * @notice Interface for PreExecution validation modules
 * @dev PreExecution modules validate messages before core execution.
 *      Modules return bools to enable composable validation logic.
 */
interface IPreExecutionModule {
    /// @notice Custom errors for pre-execution failures
    error PreExecutionFailed(bytes32 messageId, uint256 errorCode);

    /**
     * @notice Validate a message before execution
     * @dev Called by Bridge during PreExecution stage
     * @param messageId Unique message identifier
     * @param payload Message data containing type and parameters
     * @param context Execution context with validator signatures
     * @return shouldExecute True if validation passes, false to reject
     * @return errorCode If false, error code for rejection (0 = success)
     */
    function beforeExecution(bytes32 messageId, bytes calldata payload, ExecutionContext calldata context)
        external
        returns (bool shouldExecute, uint256 errorCode);
}
