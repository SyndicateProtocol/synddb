// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {ModuleCheckRegistry} from "src/ModuleCheckRegistry.sol";
import {IBridge} from "src/interfaces/IBridge.sol";
import {IWrappedNativeToken} from "src/interfaces/IWrappedNativeToken.sol";
import {ITeeKeyManager} from "src/interfaces/ITeeKeyManager.sol";
import {IAttestationVerifier} from "src/interfaces/IAttestationVerifier.sol";
import {ProcessingStage, MessageState, SequencerSignature} from "src/types/DataTypes.sol";
import {ECDSA} from "@openzeppelin/contracts/utils/cryptography/ECDSA.sol";
import {MessageHashUtils} from "@openzeppelin/contracts/utils/cryptography/MessageHashUtils.sol";

/**
 * @title Bridge
 * @notice Cross-chain message bridge for SyndDB that validates and executes sequenced messages
 * @dev Manages message lifecycle with pre/post execution validation modules, native token transfers,
 *      and TEE key management. Uses Ownable2Step for secure ownership transfer.
 */
contract Bridge is IBridge, ModuleCheckRegistry {
    mapping(bytes32 messageId => MessageState state) public messageStates;

    IWrappedNativeToken public immutable wrappedNativeToken;

    /// @notice Whether sequencer key registration requires owner approval
    bool public sequencerKeyRegistrationRestricted;

    /// @notice Whether validator key registration requires owner approval
    bool public validatorKeyRegistrationRestricted;

    /// @notice Default expiration duration for new keys (0 = never expires)
    uint256 public defaultKeyExpiration;

    event MessageInitialized(bytes32 indexed messageId, bytes payload);
    event MessageHandled(bytes32 indexed messageId, bool success);
    event NativeTokenWrapped(address indexed sender, uint256 amount);
    event NativeTokenUnwrapped(uint256 amount, address indexed target);
    event TeeKeyManagerUpdated(address indexed oldKeyManager, address indexed newKeyManager);
    event SequencerKeyRegistrationRestrictionUpdated(bool restricted);
    event ValidatorKeyRegistrationRestrictionUpdated(bool restricted);
    event DefaultKeyExpirationUpdated(uint256 expiration);

    error ZeroAddressNotAllowed();
    error InvalidSequencerSignature(address recoveredSigner);
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
     * @param _owner Address to be granted ownership
     * @param _wrappedNativeToken Address of the wrapped native token contract (e.g., WETH)
     * @param _teeKeyManager Address of the TEE key manager contract for sequencer/validator verification
     */
    constructor(address _owner, address _wrappedNativeToken, address _teeKeyManager) ModuleCheckRegistry(_owner) {
        if (_owner == address(0) || _wrappedNativeToken == address(0) || _teeKeyManager == address(0)) {
            revert ZeroAddressNotAllowed();
        }
        wrappedNativeToken = IWrappedNativeToken(_wrappedNativeToken);
        teeKeyManager = ITeeKeyManager(_teeKeyManager);
    }

    /**
     * @notice Receives native token and wraps it to wrappedNativeToken for internal accounting
     * @dev When msg.sender is the WrappedNativeToken contract itself (during unwrapping),
     *      the native token is NOT re-wrapped to prevent infinite loops.
     */
    receive() external payable {
        if (msg.sender != address(wrappedNativeToken)) {
            _wrapNativeToken(msg.value);
        }
    }

    /*//////////////////////////////////////////////////////////////
                            KEY MANAGEMENT
    //////////////////////////////////////////////////////////////*/

    /**
     * @notice Updates the TEE key manager contract address
     * @dev Only callable by owner. Use with caution as this changes which keys are considered valid.
     * @param _teeKeyManager New TEE key manager contract address
     */
    function setTeeKeyManager(address _teeKeyManager) external onlyOwner {
        if (_teeKeyManager == address(0)) {
            revert ZeroAddressNotAllowed();
        }
        address oldKeyManager = address(teeKeyManager);
        teeKeyManager = ITeeKeyManager(_teeKeyManager);
        emit TeeKeyManagerUpdated(oldKeyManager, _teeKeyManager);
    }

    /**
     * @notice Sets whether sequencer key registration requires owner approval
     * @param restricted If true, new sequencer keys require approval
     */
    function setSequencerKeyRegistrationRestricted(bool restricted) external onlyOwner {
        sequencerKeyRegistrationRestricted = restricted;
        emit SequencerKeyRegistrationRestrictionUpdated(restricted);
    }

    /**
     * @notice Sets whether validator key registration requires owner approval
     * @param restricted If true, new validator keys require approval
     */
    function setValidatorKeyRegistrationRestricted(bool restricted) external onlyOwner {
        validatorKeyRegistrationRestricted = restricted;
        emit ValidatorKeyRegistrationRestrictionUpdated(restricted);
    }

    /**
     * @notice Sets the default expiration duration for new keys
     * @param expiration Duration in seconds from registration (0 = never expires)
     */
    function setDefaultKeyExpiration(uint256 expiration) external onlyOwner {
        defaultKeyExpiration = expiration;
        emit DefaultKeyExpirationUpdated(expiration);
    }

    /**
     * @notice Registers a new sequencer key with attestation proof
     * @dev Anyone can call this. If registration is restricted, key goes to pending state.
     * @param publicValues The encoded public values from the attestation
     * @param proofBytes The SP1 proof bytes
     * @return publicKey The registered key address
     */
    function registerSequencerKey(bytes calldata publicValues, bytes calldata proofBytes)
        external
        returns (address publicKey)
    {
        uint256 expiresAt = defaultKeyExpiration == 0 ? 0 : block.timestamp + defaultKeyExpiration;
        return teeKeyManager.addSequencerKey(publicValues, proofBytes, sequencerKeyRegistrationRestricted, expiresAt);
    }

    /**
     * @notice Registers a new validator key with attestation proof
     * @dev Anyone can call this. If registration is restricted, key goes to pending state.
     * @param publicValues The encoded public values from the attestation
     * @param proofBytes The SP1 proof bytes
     * @return publicKey The registered key address
     */
    function registerValidatorKey(bytes calldata publicValues, bytes calldata proofBytes)
        external
        returns (address publicKey)
    {
        uint256 expiresAt = defaultKeyExpiration == 0 ? 0 : block.timestamp + defaultKeyExpiration;
        return teeKeyManager.addValidatorKey(publicValues, proofBytes, validatorKeyRegistrationRestricted, expiresAt);
    }

    /**
     * @notice Registers a sequencer key via signature (for keys without gas)
     * @param publicValues The encoded public values from the attestation
     * @param proofBytes The SP1 proof bytes
     * @param deadline Timestamp after which the signature expires
     * @param signature EIP-712 signature from the TEE key
     * @return publicKey The registered key address
     */
    function registerSequencerKeyWithSignature(
        bytes calldata publicValues,
        bytes calldata proofBytes,
        uint256 deadline,
        bytes calldata signature
    ) external returns (address publicKey) {
        uint256 expiresAt = defaultKeyExpiration == 0 ? 0 : block.timestamp + defaultKeyExpiration;
        return teeKeyManager.addSequencerKeyWithSignature(
            publicValues, proofBytes, deadline, signature, sequencerKeyRegistrationRestricted, expiresAt
        );
    }

    /**
     * @notice Registers a validator key via signature (for keys without gas)
     * @param publicValues The encoded public values from the attestation
     * @param proofBytes The SP1 proof bytes
     * @param deadline Timestamp after which the signature expires
     * @param signature EIP-712 signature from the TEE key
     * @return publicKey The registered key address
     */
    function registerValidatorKeyWithSignature(
        bytes calldata publicValues,
        bytes calldata proofBytes,
        uint256 deadline,
        bytes calldata signature
    ) external returns (address publicKey) {
        uint256 expiresAt = defaultKeyExpiration == 0 ? 0 : block.timestamp + defaultKeyExpiration;
        return teeKeyManager.addValidatorKeyWithSignature(
            publicValues, proofBytes, deadline, signature, validatorKeyRegistrationRestricted, expiresAt
        );
    }

    /**
     * @notice Approves a pending sequencer key
     * @param publicKey The pending key to approve
     * @param expiresAt Expiration timestamp (0 = never expires)
     */
    function approveSequencerKey(address publicKey, uint256 expiresAt) external onlyOwner {
        teeKeyManager.approveSequencerKey(publicKey, expiresAt);
    }

    /**
     * @notice Approves a pending validator key
     * @param publicKey The pending key to approve
     * @param expiresAt Expiration timestamp (0 = never expires)
     */
    function approveValidatorKey(address publicKey, uint256 expiresAt) external onlyOwner {
        teeKeyManager.approveValidatorKey(publicKey, expiresAt);
    }

    /**
     * @notice Rejects a pending sequencer key
     * @param publicKey The pending key to reject
     */
    function rejectSequencerKey(address publicKey) external onlyOwner {
        teeKeyManager.rejectSequencerKey(publicKey);
    }

    /**
     * @notice Rejects a pending validator key
     * @param publicKey The pending key to reject
     */
    function rejectValidatorKey(address publicKey) external onlyOwner {
        teeKeyManager.rejectValidatorKey(publicKey);
    }

    /**
     * @notice Removes a sequencer key
     * @param publicKey The key to remove
     */
    function removeSequencerKey(address publicKey) external onlyOwner {
        teeKeyManager.removeSequencerKey(publicKey);
    }

    /**
     * @notice Removes a validator key
     * @param publicKey The key to remove
     */
    function removeValidatorKey(address publicKey) external onlyOwner {
        teeKeyManager.removeValidatorKey(publicKey);
    }

    /**
     * @notice Sets expiration for a key
     * @param publicKey The key to update
     * @param expiresAt New expiration timestamp (0 = never expires)
     */
    function setKeyExpiration(address publicKey, uint256 expiresAt) external onlyOwner {
        teeKeyManager.setKeyExpiration(publicKey, expiresAt);
    }

    /**
     * @notice Revokes all registered keys
     */
    function revokeAllKeys() external onlyOwner {
        teeKeyManager.revokeAllKeys();
    }

    /**
     * @notice Updates the attestation verifier in the TeeKeyManager
     * @param _attestationVerifier The new attestation verifier contract
     */
    function updateAttestationVerifier(IAttestationVerifier _attestationVerifier) external onlyOwner {
        teeKeyManager.updateAttestationVerifier(_attestationVerifier);
    }

    /*//////////////////////////////////////////////////////////////
                            NATIVE TOKEN HANDLING
    //////////////////////////////////////////////////////////////*/

    /**
     * @notice Wraps any stuck native token in the bridge to wrapped native token
     * @dev Can be called by message initializer to recover stuck native token.
     * @param amount Maximum amount to wrap (will wrap min(amount, address(this).balance))
     */
    function wrapNativeToken(uint256 amount) external onlyMessageInitializer {
        uint256 balance = address(this).balance;
        uint256 amountToWrap = amount > balance ? balance : amount;

        if (amountToWrap == 0) {
            revert NoNativeTokenToWrap();
        }

        _wrapNativeToken(amountToWrap);
    }

    function _wrapNativeToken(uint256 amount) private {
        wrappedNativeToken.deposit{value: amount}();
        emit NativeTokenWrapped(msg.sender, amount);
    }

    /*//////////////////////////////////////////////////////////////
                            MESSAGE HANDLING
    //////////////////////////////////////////////////////////////*/

    /// @inheritdoc IBridge
    function initializeMessage(
        bytes32 messageId,
        address targetAddress,
        bytes calldata payload,
        SequencerSignature calldata sequencerSignature,
        uint256 nativeTokenAmount
    ) public onlyMessageInitializer {
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

        // Verify sequencer signature is from a registered TEE key
        bytes32 messageHash =
            keccak256(abi.encodePacked(messageId, targetAddress, keccak256(payload), nativeTokenAmount));
        bytes32 ethSignedHash = MessageHashUtils.toEthSignedMessageHash(messageHash);
        address signer = ECDSA.recover(ethSignedHash, sequencerSignature.signature);

        // This will revert with InvalidPublicKey if the signer is not a registered TEE sequencer key
        if (!teeKeyManager.isSequencerKeyValid(signer)) {
            revert InvalidSequencerSignature(signer);
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

    /// @inheritdoc IBridge
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

    /// @inheritdoc IBridge
    function initializeAndHandleMessage(
        bytes32 messageId,
        address targetAddress,
        bytes calldata payload,
        SequencerSignature calldata sequencerSignature,
        bytes[] calldata validatorSignatures,
        uint256 nativeTokenAmount
    ) external {
        initializeMessage(messageId, targetAddress, payload, sequencerSignature, nativeTokenAmount);

        for (uint256 i = 0; i < validatorSignatures.length; i++) {
            signMessageWithSignature(messageId, validatorSignatures[i]);
        }

        handleMessage(messageId);
    }

    /// @inheritdoc IBridge
    function isMessageCompleted(bytes32 messageId) public view returns (bool) {
        return messageStates[messageId].stage == ProcessingStage.Completed;
    }

    /// @inheritdoc IBridge
    function isMessageRejected(bytes32 messageId) public view returns (bool) {
        return messageStates[messageId].stage == ProcessingStage.Rejected;
    }

    /// @inheritdoc IBridge
    function isMessageHandled(bytes32 messageId) public view returns (bool) {
        return isMessageCompleted(messageId) || isMessageRejected(messageId);
    }

    /// @inheritdoc IBridge
    function isMessageInitialized(bytes32 messageId) public view returns (bool) {
        return messageStates[messageId].stage != ProcessingStage.NotStarted;
    }

    /*//////////////////////////////////////////////////////////////
                            BATCH OPERATIONS
    //////////////////////////////////////////////////////////////*/

    /**
     * @notice Initializes multiple messages in a single transaction
     * @dev Only callable by message initializers. All arrays must have equal length.
     */
    function batchInitializeMessage(
        bytes32[] calldata messageIds,
        address[] calldata targetAddresses,
        bytes[] calldata payloads,
        SequencerSignature[] calldata _sequencerSignatures,
        uint256[] calldata nativeTokenAmounts
    ) external onlyMessageInitializer {
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
     * @param messageIds Array of message identifiers to execute
     */
    function batchHandleMessage(bytes32[] calldata messageIds) external {
        for (uint256 i = 0; i < messageIds.length; i++) {
            handleMessage(messageIds[i]);
        }
    }
}
