// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {ModuleCheckRegistry} from "src/ModuleCheckRegistry.sol";
import {IBridge} from "src/interfaces/IBridge.sol";
import {IWrappedNativeToken} from "src/interfaces/IWrappedNativeToken.sol";
import {ProcessingStage, MessageState, SequencerSignature} from "src/types/DataTypes.sol";

/**
 * @title Bridge
 * @notice Cross-chain message bridge for SyndDB that validates and executes sequenced messages
 * @dev Manages message lifecycle with pre/post execution validation modules and native token transfers
 */
contract Bridge is IBridge, ModuleCheckRegistry {
    mapping(bytes32 messageId => MessageState state) public messageStates;
    mapping(bytes32 messageId => SequencerSignature signature) public sequencerSignatures;

    IWrappedNativeToken public immutable wrappedNativeToken;

    /**
     * @notice Emitted when a new message is initialized
     * @param messageId Unique identifier of the message
     * @param payload Encoded function call data
     */
    event MessageInitialized(bytes32 indexed messageId, bytes payload);

    /**
     * @notice Emitted when a message execution completes
     * @param messageId Unique identifier of the message
     * @param success Whether the execution succeeded
     */
    event MessageHandled(bytes32 indexed messageId, bool success);

    /**
     * @notice Emitted when native token is wrapped
     * @param sender Address that sent the native token
     * @param amount Amount of native token wrapped
     */
    event NativeTokenWrapped(address indexed sender, uint256 amount);

    /**
     * @notice Emitted when wrapped native token is unwrapped for message execution
     * @param amount Amount of native token unwrapped
     * @param target Address receiving the native token
     */
    event NativeTokenUnwrapped(uint256 amount, address indexed target);

    error MessageAlreadyInitialized(bytes32 messageId);
    error MessageNotInitialized(bytes32 messageId);
    error MessageAlreadyHandled(bytes32 messageId);
    error MessageCurrentlyProcessing(bytes32 messageId, ProcessingStage currentStage);
    error MessageExecutionFailed(bytes32 messageId, bytes returnData);
    error ArrayLengthMismatch();
    error InsufficientWrappedNativeTokenBalance(uint256 required, uint256 available);
    error NoNativeTokenToWrap();

    /**
     * @notice Initializes the bridge contract
     * @param admin Address to be granted admin privileges
     * @param _wrappedNativeToken Address of the wrapped native token contract (e.g., WETH)
     */
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
            _wrapNativeToken(msg.value);
        }
    }

    /**
     * @notice Wraps any stuck native token in the bridge to wrapped native token
     * @dev This function can be called by the sequencer to recover any native token that may be stuck in the contract.
     * It wraps up to the specified amount, limited by the contract's current native token balance.
     * This should not be needed in normal operation but provides a safety mechanism.
     * @param amount Maximum amount to wrap (will wrap min(amount, address(this).balance))
     */
    function wrapNativeToken(uint256 amount) external onlyRole(SEQUENCER_ROLE) {
        uint256 balance = address(this).balance;
        uint256 amountToWrap = amount > balance ? balance : amount;

        if (amountToWrap == 0) {
            revert NoNativeTokenToWrap();
        }

        _wrapNativeToken(amountToWrap);
    }

    /**
     * @notice Internal function to wrap native token to wrapped native token
     * @param amount Amount of native token to wrap
     */
    function _wrapNativeToken(uint256 amount) private {
        wrappedNativeToken.deposit{value: amount}();
        emit NativeTokenWrapped(msg.sender, amount);
    }

    /**
     * @notice Initializes a new cross-chain message
     * @dev Only callable by addresses with SEQUENCER_ROLE
     * @param messageId Unique identifier for the message
     * @param targetAddress Address that will receive the message call
     * @param payload Encoded function call data
     * @param sequencerSignature Signature from the trusted sequencer
     * @param nativeTokenAmount Amount of native token to transfer with the call
     */
    function initializeMessage(
        bytes32 messageId,
        address targetAddress,
        bytes calldata payload,
        SequencerSignature calldata sequencerSignature,
        uint256 nativeTokenAmount
    ) public onlyRole(SEQUENCER_ROLE) {
        _initializeMessage(messageId, targetAddress, payload, sequencerSignature, nativeTokenAmount);
    }

    /**
     * @notice Internal function to initialize a message
     * @dev Creates the message state and stores the sequencer signature
     * @param messageId Unique identifier for the message
     * @param targetAddress Address that will receive the message call
     * @param payload Encoded function call data
     * @param sequencerSignature Signature from the trusted sequencer
     * @param nativeTokenAmount Amount of native token to transfer with the call
     */
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

    /**
     * @notice Initializes and immediately executes a message in a single transaction
     * @dev Combines initialization, validator signature collection, and execution
     * @param messageId Unique identifier for the message
     * @param targetAddress Address that will receive the message call
     * @param payload Encoded function call data
     * @param sequencerSignature Signature from the trusted sequencer
     * @param validatorSignatures Array of signatures from authorized validators
     * @param nativeTokenAmount Amount of native token to transfer with the call
     */
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

    /**
     * @notice Checks if a message has been successfully completed
     * @param messageId Unique identifier of the message
     * @return bool True if the message reached the Completed stage
     */
    function isMessageCompleted(bytes32 messageId) public view returns (bool) {
        return messageStates[messageId].stage == ProcessingStage.Completed;
    }

    /**
     * @notice Checks if a message has been rejected
     * @param messageId Unique identifier of the message
     * @return bool True if the message reached the Rejected stage
     */
    function isMessageRejected(bytes32 messageId) public view returns (bool) {
        return messageStates[messageId].stage == ProcessingStage.Rejected;
    }

    /**
     * @notice Checks if a message has finished processing (completed or rejected)
     * @param messageId Unique identifier of the message
     * @return bool True if the message is in a terminal state
     */
    function isMessageHandled(bytes32 messageId) public view returns (bool) {
        return isMessageCompleted(messageId) || isMessageRejected(messageId);
    }

    /**
     * @notice Checks if a message has been initialized
     * @param messageId Unique identifier of the message
     * @return bool True if the message has been initialized
     */
    function isMessageInitialized(bytes32 messageId) public view returns (bool) {
        return messageStates[messageId].stage != ProcessingStage.NotStarted;
    }

    /*//////////////////////////////////////////////////////////////
                            BATCH OPERATIONS
    //////////////////////////////////////////////////////////////*/

    /**
     * @notice Initializes multiple messages in a single transaction
     * @dev Only callable by addresses with SEQUENCER_ROLE. All arrays must have equal length.
     *      If any message initialization fails, the entire batch will revert atomically.
     * @param messageIds Array of unique message identifiers
     * @param targetAddresses Array of addresses that will receive message calls
     * @param payloads Array of encoded function call data
     * @param _sequencerSignatures Array of sequencer signatures
     * @param nativeTokenAmounts Array of native token amounts to transfer
     */
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

    /**
     * @notice Executes multiple previously initialized messages in a single transaction
     * @dev If any message execution fails, the entire batch will revert and no partial batch will be committed.
     *      This ensures atomic execution of all messages in the batch.
     * @param messageIds Array of message identifiers to execute
     */
    function batchHandleMessage(bytes32[] calldata messageIds) external {
        for (uint256 i = 0; i < messageIds.length; i++) {
            handleMessage(messageIds[i]);
        }
    }
}
