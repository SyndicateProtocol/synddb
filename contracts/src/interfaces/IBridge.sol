// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {ValidatorSignatures} from "src/types/DataTypes.sol";

interface IBridge {
    function initializeMessage(
        bytes32 messageId,
        bytes calldata payload,
        ValidatorSignatures calldata executionSignatures
    ) external;

    function executeMessage(bytes32 messageId) external;

    function initializeAndExecuteMessage(
        bytes32 messageId,
        bytes calldata payload,
        ValidatorSignatures calldata executionSignatures
    ) external;

    function isMessageExecuted(bytes32 messageId) external view returns (bool);
}
