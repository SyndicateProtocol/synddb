// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {ModuleCheckRegistry} from "src/ModuleCheckRegistry.sol";

import {IBridge} from "src/interfaces/IBridge.sol";
import {ProcessingStage, MessageState, ValidatorSignatures} from "src/types/DataTypes.sol";

contract Bridge is IBridge, ModuleCheckRegistry {
    mapping(bytes32 messageId => MessageState state) public messageStates;
    mapping(bytes32 messageId => ValidatorSignatures sigs) public messageSignatures;

    event MessageInitialized(bytes32 indexed messageId, bytes payload);
    event MessageExecuted(bytes32 indexed messageId, bool success);

    error MessageAlreadyInitialized(bytes32 messageId);
    error MessageNotInitialized(bytes32 messageId);
    error MessageAlreadyExecuted(bytes32 messageId);

    function initializeMessage(
        bytes32 messageId,
        bytes calldata payload,
        ValidatorSignatures calldata executionSignatures
    ) public {
        if (messageStates[messageId].stage != ProcessingStage.NotStarted) {
            revert MessageAlreadyInitialized(messageId);
        }

        messageStates[messageId] =
            MessageState({messageId: messageId, stage: ProcessingStage.PreExecution, payload: payload});

        messageSignatures[messageId] = executionSignatures;

        emit MessageInitialized(messageId, payload);
    }

    function executeMessage(bytes32 messageId) public {
        MessageState storage state = messageStates[messageId];

        if (state.stage == ProcessingStage.NotStarted) {
            revert MessageNotInitialized(messageId);
        }

        if (state.stage == ProcessingStage.Completed) {
            revert MessageAlreadyExecuted(messageId);
        }

        ValidatorSignatures memory sigs = messageSignatures[messageId];

        _validatePreModules(ProcessingStage.PreExecution, state.payload, sigs);

        state.stage = ProcessingStage.Executing;

        // How does execution entail? Is it sending the call to a cross chain contract?
        // some sort of execution logic would go here

        state.stage = ProcessingStage.PostExecution;

        _validatePostModules(ProcessingStage.PostExecution, state.payload, sigs);

        state.stage = ProcessingStage.Completed;

        emit MessageExecuted(messageId, true);
    }

    function initializeAndExecuteMessage(
        bytes32 messageId,
        bytes calldata payload,
        ValidatorSignatures calldata executionSignatures
    ) external {
        initializeMessage(messageId, payload, executionSignatures);
        executeMessage(messageId);
    }

    function isMessageExecuted(bytes32 messageId) external view returns (bool) {
        return messageStates[messageId].stage == ProcessingStage.Completed;
    }
}
