// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

interface IBridge {
    enum ProcessingStage {
        NotStarted,
        PreExecution,
        Executing,
        PostExecution,
        Completed,
        Rejected
    }

    struct MessageState {
        bytes32 messageId;
        ProcessingStage stage;
        bytes payload;
    }

    struct ValidatorSignatures {
        bytes[] validatorSignatures;
        uint256 submittedAt;
    }

    function initializeMessage(bytes32 messageId, bytes calldata payload, ValidatorSignatures calldata executionSigs)
        external;

    function executeMessage(bytes32 messageId) external;

    function initializeAndExecuteMessage(
        bytes32 messageId,
        bytes calldata payload,
        ValidatorSignatures calldata executionSigs
    ) external;

    function isMessageExecuted(bytes32 messageId) external view returns (bool);

    /*//////////////////////////////////////////////////////////////
                            VALIDATION FUNCTIONS
    //////////////////////////////////////////////////////////////*/

    function addPreModule(address module) external;

    function addPostModule(address module) external;

    function removePreModule(address module) external;

    function removePostModule(address module) external;

    function getPreModules() external view returns (address[] memory);

    function getPostModules() external view returns (address[] memory);

}
