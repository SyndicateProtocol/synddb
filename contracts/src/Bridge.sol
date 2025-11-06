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
    event MessageHandled(bytes32 indexed messageId, bool success);

    error MessageAlreadyInitialized(bytes32 messageId);
    error MessageNotInitialized(bytes32 messageId);
    error MessageAlreadyHandled(bytes32 messageId);
    error MessageExecutionFailed(bytes32 messageId, bytes returnData);
    error ArrayLengthMismatch();

    constructor(address admin) ModuleCheckRegistry(admin) {}

    function initializeMessage(
        bytes32 messageId,
        address targetAddress,
        bytes calldata payload,
        SequencerSignature calldata sequencerSignature
    ) public onlyRole(SEQUENCER_ROLE) {
        if (messageStates[messageId].stage != ProcessingStage.NotStarted) {
            revert MessageAlreadyInitialized(messageId);
        }

        messageStates[messageId] = MessageState({
            messageId: messageId,
            targetAddress: targetAddress,
            stage: ProcessingStage.PreExecution,
            payload: payload,
            createdAt: block.timestamp
        });

        sequencerSignatures[messageId] = sequencerSignature;

        emit MessageInitialized(messageId, payload);
    }

    function handleMessage(bytes32 messageId) public nonReentrant {
        MessageState storage state = messageStates[messageId];

        if (state.stage == ProcessingStage.NotStarted) {
            revert MessageNotInitialized(messageId);
        }

        if (state.stage == ProcessingStage.Completed) {
            revert MessageAlreadyHandled(messageId);
        }

        SequencerSignature memory signature = sequencerSignatures[messageId];

        _validatePreModules(messageId, ProcessingStage.PreExecution, state.payload, signature);

        state.stage = ProcessingStage.Executing;

        (bool success, bytes memory returnData) = state.targetAddress.call(state.payload);

        if (!success) {
            revert MessageExecutionFailed(messageId, returnData);
        }

        state.stage = ProcessingStage.PostExecution;

        _validatePostModules(messageId, ProcessingStage.PostExecution, state.payload, signature);

        state.stage = ProcessingStage.Completed;

        emit MessageHandled(messageId, true);
    }

    function initializeAndHandleMessage(
        bytes32 messageId,
        address targetAddress,
        bytes calldata payload,
        SequencerSignature calldata sequencerSignature,
        bytes[] calldata validatorSignatures
    ) external {
        initializeMessage(messageId, targetAddress, payload, sequencerSignature);

        // collect validator signatures and verify them
        for (uint256 i = 0; i < validatorSignatures.length; i++) {
            signMessageWithSignature(messageId, validatorSignatures[i]);
        }

        handleMessage(messageId);
    }

    function isMessageHandled(bytes32 messageId) external view returns (bool) {
        return messageStates[messageId].stage == ProcessingStage.Completed;
    }

    /*//////////////////////////////////////////////////////////////
                            BATCH OPERATIONS
    //////////////////////////////////////////////////////////////*/

    function batchInitializeMessage(
        bytes32[] calldata messageIds,
        address[] calldata targetAddresses,
        bytes[] calldata payloads,
        SequencerSignature[] calldata _sequencerSignatures
    ) external onlyRole(SEQUENCER_ROLE) {
        if (
            messageIds.length != targetAddresses.length || messageIds.length != payloads.length
                || messageIds.length != _sequencerSignatures.length
        ) {
            revert ArrayLengthMismatch();
        }

        for (uint256 i = 0; i < messageIds.length; i++) {
            initializeMessage(messageIds[i], targetAddresses[i], payloads[i], _sequencerSignatures[i]);
        }
    }

    function batchHandleMessage(bytes32[] calldata messageIds) external {
        for (uint256 i = 0; i < messageIds.length; i++) {
            handleMessage(messageIds[i]);
        }
    }
}
