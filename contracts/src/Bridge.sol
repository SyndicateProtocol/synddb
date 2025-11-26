// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {ModuleCheckRegistry} from "src/ModuleCheckRegistry.sol";
import {IBridge} from "src/interfaces/IBridge.sol";
import {IWrappedNativeToken} from "src/interfaces/IWrappedNativeToken.sol";
import {ProcessingStage, MessageState, SequencerSignature} from "src/types/DataTypes.sol";

contract Bridge is IBridge, ModuleCheckRegistry {
    mapping(bytes32 messageId => MessageState state) public messageStates;
    mapping(bytes32 messageId => SequencerSignature signature) public sequencerSignatures;

    IWrappedNativeToken public immutable wrappedNativeToken;

    event MessageInitialized(bytes32 indexed messageId, bytes payload);
    event MessageHandled(bytes32 indexed messageId, bool success);
    event NativeTokenWrapped(address indexed sender, uint256 amount);
    event NativeTokenUnwrapped(uint256 amount, address indexed target);

    error MessageAlreadyInitialized(bytes32 messageId);
    error MessageNotInitialized(bytes32 messageId);
    error MessageAlreadyHandled(bytes32 messageId);
    error MessageCurrentlyProcessing(bytes32 messageId, ProcessingStage currentStage);
    error MessageExecutionFailed(bytes32 messageId, bytes returnData);
    error ArrayLengthMismatch();
    error InsufficientWrappedNativeTokenBalance(uint256 required, uint256 available);

    constructor(address admin, address _wrappedNativeToken) ModuleCheckRegistry(admin) {
        wrappedNativeToken = IWrappedNativeToken(_wrappedNativeToken);
    }

    /**
     * @notice Receives native native token and wraps it to wrappedNativeToken for internal accounting
     * @dev This function is intentionally public and allows anyone to send native token to the bridge.
     * The native token is immediately wrapped to wrappedNativeToken for consistent accounting and balance tracking.
     *
     * When msg.sender is the WrappedNativeToken contract itself (during unwrapping in handleMessage),
     * the native token is NOT re-wrapped to prevent infinite loops.
     */
    receive() external payable {
        // Only wrap native token if it's not coming from WrappedNativeToken unwrapping
        if (msg.sender != address(wrappedNativeToken)) {
            wrappedNativeToken.deposit{value: msg.value}();
            emit NativeTokenWrapped(msg.sender, msg.value);
        }
    }

    function initializeMessage(
        bytes32 messageId,
        address targetAddress,
        bytes calldata payload,
        SequencerSignature calldata sequencerSignature,
        uint256 nativeTokenAmount
    ) public onlyRole(SEQUENCER_ROLE) {
        _initializeMessage(messageId, targetAddress, payload, sequencerSignature, nativeTokenAmount);
    }

    function _initializeMessage(
        bytes32 messageId,
        address targetAddress,
        bytes calldata payload,
        SequencerSignature calldata sequencerSignature,
        uint256 nativeTokenAmount
    ) internal {
        if (isMessageInitialized(messageId)) {
            revert MessageAlreadyInitialized(messageId);
        }

        messageStates[messageId] = MessageState({
            messageId: messageId,
            targetAddress: targetAddress,
            stage: ProcessingStage.PreExecution,
            payload: payload,
            createdAt: block.timestamp,
            nativeTokenAmount: nativeTokenAmount
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

        if (state.nativeTokenAmount > 0) {
            uint256 wrappedNativeTokenBalance = wrappedNativeToken.balanceOf(address(this));
            if (wrappedNativeTokenBalance < state.nativeTokenAmount) {
                revert InsufficientWrappedNativeTokenBalance(state.nativeTokenAmount, wrappedNativeTokenBalance);
            }

            wrappedNativeToken.withdraw(state.nativeTokenAmount);
            emit NativeTokenUnwrapped(state.nativeTokenAmount, state.targetAddress);
        }

        (bool success, bytes memory returnData) =
            state.targetAddress.call{value: state.nativeTokenAmount}(state.payload);

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
        bytes[] calldata validatorSignatures,
        uint256 nativeTokenAmount
    ) external {
        initializeMessage(messageId, targetAddress, payload, sequencerSignature, nativeTokenAmount);

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
        SequencerSignature[] calldata _sequencerSignatures,
        uint256[] calldata nativeTokenAmounts
    ) external onlyRole(SEQUENCER_ROLE) {
        if (
            messageIds.length != targetAddresses.length || messageIds.length != payloads.length
                || messageIds.length != _sequencerSignatures.length || messageIds.length != nativeTokenAmounts.length
        ) {
            revert ArrayLengthMismatch();
        }

        for (uint256 i = 0; i < messageIds.length; i++) {
            _initializeMessage(
                messageIds[i], targetAddresses[i], payloads[i], _sequencerSignatures[i], nativeTokenAmounts[i]
            );
        }
    }

    function batchHandleMessage(bytes32[] calldata messageIds) external {
        for (uint256 i = 0; i < messageIds.length; i++) {
            handleMessage(messageIds[i]);
        }
    }
}
