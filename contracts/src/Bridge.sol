// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {ModuleCheckRegistry} from "src/ModuleCheckRegistry.sol";

import {IBridge} from "src/interfaces/IBridge.sol";
import {ProcessingStage, MessageState, SequencerSignature} from "src/types/DataTypes.sol";
import {ReentrancyGuard} from "@openzeppelin/contracts/utils/ReentrancyGuard.sol";

contract Bridge is IBridge, ModuleCheckRegistry, ReentrancyGuard {
    mapping(bytes32 messageId => MessageState state) public messageStates;
    mapping(bytes32 messageId => SequencerSignature signature) public sequencerSignatures;

    event MessageInitialized(bytes32 indexed messageId, bytes payload);
    event MessageExecuted(bytes32 indexed messageId, bool success);

    error MessageAlreadyInitialized(bytes32 messageId);
    error MessageNotInitialized(bytes32 messageId);
    error MessageAlreadyExecuted(bytes32 messageId);

    constructor(address admin) ModuleCheckRegistry(admin) {}

    function initializeMessage(
        bytes32 messageId,
        bytes calldata payload,
        SequencerSignature calldata sequencerSignature
    ) public onlyRole(SEQUENCER_ROLE) {
        if (messageStates[messageId].stage != ProcessingStage.NotStarted) {
            revert MessageAlreadyInitialized(messageId);
        }

        messageStates[messageId] = MessageState({
            messageId: messageId,
            stage: ProcessingStage.PreExecution,
            payload: payload,
            createdAt: block.timestamp
        });

        sequencerSignatures[messageId] = sequencerSignature;

        emit MessageInitialized(messageId, payload);
    }

    function executeMessage(bytes32 messageId) public nonReentrant {
        MessageState storage state = messageStates[messageId];

        if (state.stage == ProcessingStage.NotStarted) {
            revert MessageNotInitialized(messageId);
        }

        if (state.stage == ProcessingStage.Completed) {
            revert MessageAlreadyExecuted(messageId);
        }

        SequencerSignature memory signature = sequencerSignatures[messageId];

        _validatePreModules(ProcessingStage.PreExecution, state.payload, signature);

        state.stage = ProcessingStage.Executing;

        // How does execution entail? Is it sending the call to a cross chain contract?
        // some sort of execution logic would go here

        state.stage = ProcessingStage.PostExecution;

        _validatePostModules(ProcessingStage.PostExecution, state.payload, signature);

        state.stage = ProcessingStage.Completed;

        emit MessageExecuted(messageId, true);
    }

    function initializeAndExecuteMessage(
        bytes32 messageId,
        bytes calldata payload,
        SequencerSignature calldata sequencerSignature
    ) external {
        initializeMessage(messageId, payload, sequencerSignature);
        executeMessage(messageId);
    }

    function isMessageExecuted(bytes32 messageId) external view returns (bool) {
        return messageStates[messageId].stage == ProcessingStage.Completed;
    }
}
