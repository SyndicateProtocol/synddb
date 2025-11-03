// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

/**
 * @title DataTypes
 * @notice Shared data structures used across the Bridge system
 */

/**
 * @notice Execution context containing validator signatures and metadata
 * @dev This struct is passed to all modules and contains the necessary
 *      information to validate and process messages
 * @param initiator Address that initiated the message
 * @param validatorSignatures Array of validator signatures for the message
 * @param timestamp Timestamp when the message was created
 */
struct ExecutionContext {
    address initiator;
    bytes[] validatorSignatures;
    uint256 timestamp;
}
