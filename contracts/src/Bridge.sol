// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {ModuleCheckRegistry} from "src/ModuleCheckRegistry.sol";

import {IBridge} from "src/interfaces/IBridge.sol";
import {ProcessingStage, MessageState, SequencerSignature} from "src/types/DataTypes.sol";

interface IWETH {
    function deposit() external payable;
}

contract Bridge is IBridge, ModuleCheckRegistry {
    mapping(bytes32 messageId => MessageState state) public messageStates;
    mapping(bytes32 messageId => SequencerSignature signature) public sequencerSignatures;

    IWETH public immutable weth;

    event MessageInitialized(bytes32 indexed messageId, bytes payload);
    event MessageHandled(bytes32 indexed messageId, bool success);
    event ETHWrapped(address indexed sender, uint256 amount);

    error MessageAlreadyInitialized(bytes32 messageId);
    error MessageNotInitialized(bytes32 messageId);
    error MessageAlreadyHandled(bytes32 messageId);
    error MessageCurrentlyProcessing(bytes32 messageId, ProcessingStage currentStage);
    error MessageExecutionFailed(bytes32 messageId, bytes returnData);
    error ArrayLengthMismatch();

    constructor(address admin, address _weth) ModuleCheckRegistry(admin) {
        weth = IWETH(_weth);
    }

    receive() external payable {
        weth.deposit{value: msg.value}();
        emit ETHWrapped(msg.sender, msg.value);
    }

    function initializeMessage(
        bytes32 messageId,
        address targetAddress,
        bytes calldata payload,
        SequencerSignature calldata sequencerSignature
    ) public onlyRole(SEQUENCER_ROLE) {
        if (isMessageInitialized(messageId)) {
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

    /**
     * @notice Processes a cross-chain message by executing its payload and validating pre/post execution modules
     * @dev This function allows reentrancy for composability but prevents re-processing the same message via stage
     * checks.
     *
     * WARNING: Message handlers should be carefully designed to handle reentrant calls. Avoid relying on contract state that could change during execution.
     * The bridge allows cross-message reentrancy to enable composable cross-chain operations, but same-message reentrancy is blocked.
     *
     * @param messageId The unique identifier of the message to process
     */
    function handleMessage(bytes32 messageId) public {
        MessageState storage state = messageStates[messageId];

        if (state.stage == ProcessingStage.NotStarted) {
            revert MessageNotInitialized(messageId);
        }

        if (isMessageHandled(messageId)) {
            revert MessageAlreadyHandled(messageId);
        }

        if (state.stage != ProcessingStage.PreExecution) {
            revert MessageCurrentlyProcessing(messageId, state.stage);
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

    function isMessageCompleted(bytes32 messageId) public view returns (bool) {
        return messageStates[messageId].stage == ProcessingStage.Completed;
    }

    function isMessageRejected(bytes32 messageId) public view returns (bool) {
        return messageStates[messageId].stage == ProcessingStage.Rejected;
    }

    function isMessageHandled(bytes32 messageId) public view returns (bool) {
        return isMessageCompleted(messageId) || isMessageRejected(messageId);
    }

    function isMessageInitialized(bytes32 messageId) public view returns (bool) {
        return messageStates[messageId].stage != ProcessingStage.NotStarted;
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
