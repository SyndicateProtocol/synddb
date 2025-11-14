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
    error OnlySelfCall();

    constructor(address admin, address _wrappedNativeToken) ModuleCheckRegistry(admin) {
        wrappedNativeToken = IWrappedNativeToken(_wrappedNativeToken);
    }

    receive() external payable {
        // Only wrap ETH if it's not coming from the WETH contract itself
        // (to avoid infinite loop during withdrawAndCall)
        if (msg.sender != address(wrappedNativeToken)) {
            wrappedNativeToken.deposit{value: msg.value}();
            emit NativeTokenWrapped(msg.sender, msg.value);
        }
    }

    /**
     * @notice Withdraws WETH to native ETH and forwards to target contract with calldata
     * @dev Can only be called by the Bridge itself (via handleMessage)
     *      This enables the Bridge to interact with contracts that require native ETH payment
     *
     * FLOW EXPLANATION:
     * ================
     * Problem: Bridge holds WETH, but some contracts require native ETH.
     *
     * Solution: This function unwraps WETH → ETH and forwards to target in one call.
     *
     * Example Flow:
     * 1. User deposits ETH → Bridge wraps it to WETH (via receive())
     * 2. Sequencer calls initializeMessage with:
     *    - targetAddress: address(bridge) (self-call)
     *    - payload: encoded withdrawAndCall(amount, tokenContract, mintCalldata)
     * 3. Validators sign the message
     * 4. handleMessage executes → Bridge calls itself → withdrawAndCall:
     *    a. Unwraps WETH → ETH
     *    b. Forwards ETH to token contract with mint calldata
     * 5. Token contract receives ETH and processes the call
     *
     * Security: Only callable by Bridge itself to prevent unauthorized WETH withdrawals
     *
     * @param amount Amount of WETH to unwrap
     * @param target Target contract to call with native ETH
     * @param data Calldata to forward to target
     * @return returnData Return data from the target call
     */
    function withdrawAndCall(uint256 amount, address payable target, bytes calldata data)
        external
        returns (bytes memory returnData)
    {
        // Only allow Bridge to call itself (via handleMessage)
        if (msg.sender != address(this)) {
            revert OnlySelfCall();
        }

        wrappedNativeToken.withdraw(amount);

        emit NativeTokenUnwrapped(amount, target);

        (bool success, bytes memory result) = target.call{value: amount}(data);
        if (!success) {
            revert MessageExecutionFailed(bytes32(0), result);
        }

        return result;
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
