// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {AccessControl} from "@openzeppelin/contracts/access/AccessControl.sol";
import {EnumerableSet} from "@openzeppelin/contracts/utils/structs/EnumerableSet.sol";
import {ReentrancyGuard} from "@openzeppelin/contracts/utils/ReentrancyGuard.sol";
import {Pausable} from "@openzeppelin/contracts/utils/Pausable.sol";

import {IMessageBridge} from "src/interfaces/IMessageBridge.sol";
import {IMessageTypeRegistry} from "src/interfaces/IMessageTypeRegistry.sol";
import {IApplicationRegistry} from "src/interfaces/IApplicationRegistry.sol";
import {IWrappedNativeToken} from "src/interfaces/IWrappedNativeToken.sol";

import {
    MessageStage,
    MessageStateV2,
    ApplicationConfig,
    ValidatorInfo,
    MessageTypeConfig,
    Rejection
} from "src/types/DataTypes.sol";

import {
    NotPrimaryValidator,
    NotValidator,
    ValidatorNotAuthorized,
    MessageNotInitialized,
    MessageAlreadyInitialized,
    MessageNotPending,
    MessageNotReady,
    MessageAlreadyTerminal,
    InvalidNonce,
    MessageIsExpired,
    CannotExpire,
    NotYetExpired,
    TimestampOutOfRange,
    AlreadySigned,
    AlreadyRejected,
    ProposalAlreadyRejected,
    InvalidSignature,
    InvalidMessageId,
    ExecutionFailed,
    InsufficientWETHBalance,
    MessageTypeNotRegistered,
    MessageTypeDisabled,
    MessageTypeAlreadyRegistered,
    ApplicationNotRegistered,
    ApplicationNotActive,
    ApplicationAlreadyRegistered,
    InvalidThreshold,
    ThresholdNotReached,
    InvalidAddress,
    InvalidAttestation,
    BatchSizeExceeded,
    StorageRefTooLong,
    ArrayLengthMismatch
} from "src/types/Errors.sol";

import {SignatureVerifier} from "src/libraries/SignatureVerifier.sol";
import {NonceManager} from "src/libraries/NonceManager.sol";

/**
 * @title MessageBridge
 * @notice Multi-validator Message Passing Bridge for cross-chain message validation and execution
 * @dev Implements Primary/Witness validator model with M-of-N threshold signing
 */
contract MessageBridge is IMessageBridge, IMessageTypeRegistry, IApplicationRegistry, AccessControl, ReentrancyGuard, Pausable {
    using EnumerableSet for EnumerableSet.AddressSet;
    using NonceManager for mapping(bytes32 => uint64);

    // ============================================================
    // CONSTANTS
    // ============================================================

    /// @notice Maximum clock drift tolerance for timestamps (1 minute)
    uint256 public constant MAX_CLOCK_DRIFT = 60;

    /// @notice Default message expiration (24 hours)
    uint64 public constant DEFAULT_EXPIRATION = 86400;

    /// @notice Maximum messages per batch operation
    uint256 public constant MAX_BATCH_SIZE = 50;

    /// @notice Maximum length for storage reference URIs
    uint256 public constant MAX_STORAGE_REF_LENGTH = 500;

    /// @notice Minimum signature threshold
    uint256 public constant MIN_THRESHOLD = 1;

    // ============================================================
    // ROLES
    // ============================================================

    /// @notice Role for registering message types and applications
    bytes32 public constant REGISTRY_ROLE = keccak256("REGISTRY_ROLE");

    /// @notice Role for managing validators
    bytes32 public constant VALIDATOR_MANAGER_ROLE = keccak256("VALIDATOR_MANAGER_ROLE");

    // ============================================================
    // IMMUTABLES
    // ============================================================

    /// @notice EIP-712 domain separator
    bytes32 public immutable DOMAIN_SEPARATOR;

    /// @notice Wrapped native token (WETH)
    IWrappedNativeToken public immutable weth;

    // ============================================================
    // STATE - Configuration
    // ============================================================

    /// @notice Minimum number of signatures required for execution
    uint256 public signatureThreshold;

    /// @notice Minimum required TEE version for validators
    string public minimumTeeVersion;

    // ============================================================
    // STATE - Messages
    // ============================================================

    /// @notice Message states by ID
    mapping(bytes32 => MessageStateV2) private _messageStates;

    /// @notice Validator signatures by message ID and validator address
    mapping(bytes32 => mapping(address => bool)) public hasValidatorSigned;

    /// @notice Validator rejections by message ID and validator address
    mapping(bytes32 => mapping(address => bool)) private _hasValidatorRejected;

    /// @notice Rejection records by message ID
    mapping(bytes32 => Rejection[]) private _messageRejections;

    /// @notice Rejected proposals by message ID (before initialization)
    mapping(bytes32 => bool) public proposalRejected;

    // ============================================================
    // STATE - Applications
    // ============================================================

    /// @notice Application configurations by domain
    mapping(bytes32 => ApplicationConfig) private _applicationConfigs;

    /// @notice Last consumed nonce by domain
    mapping(bytes32 => uint64) private _lastNonces;

    // ============================================================
    // STATE - Validators
    // ============================================================

    /// @notice Validator info by address
    mapping(address => ValidatorInfo) private _validatorInfo;

    /// @notice Set of active Witness Validators
    EnumerableSet.AddressSet private _witnessValidators;

    // ============================================================
    // STATE - Message Types
    // ============================================================

    /// @notice Message type configurations
    mapping(string => MessageTypeConfig) private _messageTypes;

    /// @notice Reverse lookup: selector to message type
    mapping(bytes4 => string) private _selectorToMessageType;

    // ============================================================
    // CONSTRUCTOR
    // ============================================================

    /**
     * @notice Initialize the MessageBridge
     * @param admin Address to receive admin role
     * @param _weth Wrapped native token address
     * @param _signatureThreshold Initial signature threshold
     */
    constructor(address admin, address _weth, uint256 _signatureThreshold) {
        if (admin == address(0)) revert InvalidAddress("admin");
        if (_weth == address(0)) revert InvalidAddress("weth");
        if (_signatureThreshold < MIN_THRESHOLD) revert InvalidThreshold(_signatureThreshold, 0);

        _grantRole(DEFAULT_ADMIN_ROLE, admin);
        _grantRole(REGISTRY_ROLE, admin);
        _grantRole(VALIDATOR_MANAGER_ROLE, admin);

        weth = IWrappedNativeToken(_weth);
        signatureThreshold = _signatureThreshold;

        DOMAIN_SEPARATOR = SignatureVerifier.computeDomainSeparator(block.chainid, address(this));
    }

    // ============================================================
    // RECEIVE
    // ============================================================

    /**
     * @notice Receive ETH and wrap to WETH
     * @dev Does not re-wrap when receiving from WETH unwrap
     */
    receive() external payable {
        if (msg.sender != address(weth)) {
            weth.deposit{value: msg.value}();
        }
    }

    // ============================================================
    // MESSAGE LIFECYCLE - INITIALIZATION
    // ============================================================

    /// @inheritdoc IMessageBridge
    function initializeMessage(
        bytes32 messageId,
        string calldata messageType,
        bytes calldata calldata_,
        bytes32 metadataHash,
        string calldata storageRef,
        uint64 nonce,
        uint64 timestamp,
        bytes32 domain
    ) external payable whenNotPaused {
        _initializeMessage(messageId, messageType, calldata_, metadataHash, storageRef, nonce, timestamp, domain);
    }

    /// @inheritdoc IMessageBridge
    function initializeAndSign(
        bytes32 messageId,
        string calldata messageType,
        bytes calldata calldata_,
        bytes32 metadataHash,
        string calldata storageRef,
        uint64 nonce,
        uint64 timestamp,
        bytes32 domain,
        bytes calldata signature
    ) external payable whenNotPaused {
        _initializeMessage(messageId, messageType, calldata_, metadataHash, storageRef, nonce, timestamp, domain);
        _signMessage(messageId, signature);
    }

    function _initializeMessage(
        bytes32 messageId,
        string calldata messageType,
        bytes calldata calldata_,
        bytes32 metadataHash,
        string calldata storageRef,
        uint64 nonce,
        uint64 timestamp,
        bytes32 domain
    ) internal {
        // 1. Verify caller is Primary Validator for this domain
        ApplicationConfig storage config = _applicationConfigs[domain];
        if (config.primaryValidator == address(0)) revert ApplicationNotRegistered(domain);
        if (!config.active) revert ApplicationNotActive(domain);
        if (config.primaryValidator != msg.sender) revert NotPrimaryValidator(domain, msg.sender);

        // 2. Verify message type is registered and enabled
        MessageTypeConfig storage typeConfig = _messageTypes[messageType];
        if (typeConfig.target == address(0)) revert MessageTypeNotRegistered(messageType);
        if (!typeConfig.enabled) revert MessageTypeDisabled(messageType);

        // 3. Verify message not already initialized
        if (_messageStates[messageId].stage != MessageStage.NotInitialized) {
            revert MessageAlreadyInitialized(messageId);
        }

        // 4. Verify proposal not rejected
        if (proposalRejected[messageId]) revert ProposalAlreadyRejected(messageId);

        // 5. Validate and consume nonce
        _lastNonces.validateAndConsume(domain, nonce);

        // 6. Validate timestamp freshness (avoiding underflow)
        // Message timestamp should be within MAX_CLOCK_DRIFT of current time
        if (
            block.timestamp + MAX_CLOCK_DRIFT < timestamp || block.timestamp > timestamp + MAX_CLOCK_DRIFT
        ) {
            revert TimestampOutOfRange(timestamp, block.timestamp, MAX_CLOCK_DRIFT);
        }

        // 7. Validate storage ref length
        if (bytes(storageRef).length > MAX_STORAGE_REF_LENGTH) {
            revert StorageRefTooLong(bytes(storageRef).length, MAX_STORAGE_REF_LENGTH);
        }

        // 8. Verify message ID matches computed value
        bytes32 computed = computeMessageId(messageType, calldata_, metadataHash, nonce, timestamp, domain);
        if (messageId != computed) revert InvalidMessageId(messageId, computed);

        // 9. Handle value (wrap ETH to WETH if sent)
        uint256 value = msg.value;
        if (value > 0) {
            weth.deposit{value: value}();
        }

        // 10. Store message state
        _messageStates[messageId] = MessageStateV2({
            stage: MessageStage.Pending,
            messageType: messageType,
            calldata_: calldata_,
            metadataHash: metadataHash,
            storageRef: storageRef,
            value: value,
            nonce: nonce,
            timestamp: timestamp,
            domain: domain,
            primaryValidator: msg.sender,
            signaturesCollected: 0,
            rejectionsCollected: 0
        });

        emit MessageInitialized(messageId, domain, msg.sender, messageType, storageRef);
    }

    // ============================================================
    // MESSAGE LIFECYCLE - SIGNING
    // ============================================================

    /// @inheritdoc IMessageBridge
    function signMessage(bytes32 messageId, bytes calldata signature) external whenNotPaused {
        _signMessage(messageId, signature);
    }

    function _signMessage(bytes32 messageId, bytes calldata signature) internal {
        MessageStateV2 storage state = _messageStates[messageId];

        // 1. Verify message is pending
        if (state.stage != MessageStage.Pending) {
            revert MessageNotPending(messageId, state.stage);
        }

        // 2. Verify not expired
        if (_isExpired(messageId)) {
            revert MessageIsExpired(messageId, state.timestamp + _applicationConfigs[state.domain].expirationSeconds);
        }

        // 3. Verify signature and get signer
        address signer = SignatureVerifier.verifySignature(DOMAIN_SEPARATOR, messageId, state, signature);

        // 4. Verify signer is an active validator
        if (!_validatorInfo[signer].active) {
            revert ValidatorNotAuthorized(signer);
        }

        // 5. Verify not already signed
        if (hasValidatorSigned[messageId][signer]) {
            revert AlreadySigned(messageId, signer);
        }

        // 6. Record signature
        hasValidatorSigned[messageId][signer] = true;
        state.signaturesCollected++;

        emit SignatureSubmitted(messageId, signer, state.signaturesCollected);

        // 7. Check if threshold reached
        if (state.signaturesCollected >= signatureThreshold) {
            state.stage = MessageStage.Ready;
            emit ThresholdReached(messageId, state.signaturesCollected);
        }
    }

    // ============================================================
    // MESSAGE LIFECYCLE - EXECUTION
    // ============================================================

    /// @inheritdoc IMessageBridge
    function executeMessage(bytes32 messageId) external nonReentrant whenNotPaused {
        MessageStateV2 storage state = _messageStates[messageId];

        // 1. Verify message is ready
        if (state.stage != MessageStage.Ready) {
            revert MessageNotReady(messageId, state.stage);
        }

        // 2. Verify not expired
        if (_isExpired(messageId)) {
            revert MessageIsExpired(messageId, state.timestamp + _applicationConfigs[state.domain].expirationSeconds);
        }

        // 3. Get target from message type config
        MessageTypeConfig storage typeConfig = _messageTypes[state.messageType];
        address target = typeConfig.target;

        // 4. Transition to PreExecution
        state.stage = MessageStage.PreExecution;

        // 5. TODO: Run pre-execution modules (Phase 7)

        // 6. Transition to Executing
        state.stage = MessageStage.Executing;

        // 7. Handle WETH unwrap if value > 0
        if (state.value > 0) {
            uint256 wethBalance = weth.balanceOf(address(this));
            if (wethBalance < state.value) {
                revert InsufficientWETHBalance(state.value, wethBalance);
            }
            weth.withdraw(state.value);
        }

        // 8. Execute the call
        (bool success, bytes memory returnData) = target.call{value: state.value}(state.calldata_);

        // 9. Transition to PostExecution
        state.stage = MessageStage.PostExecution;

        // 10. TODO: Run post-execution modules (Phase 7)

        // 11. Final state
        if (success) {
            state.stage = MessageStage.Completed;
            emit MessageExecuted(messageId, target, true);
        } else {
            state.stage = MessageStage.Failed;
            emit MessageFailed(messageId, returnData);
            revert ExecutionFailed(messageId, returnData);
        }
    }

    // ============================================================
    // MESSAGE LIFECYCLE - EXPIRATION
    // ============================================================

    /// @inheritdoc IMessageBridge
    function expireMessage(bytes32 messageId) external {
        MessageStateV2 storage state = _messageStates[messageId];

        // Can only expire Pending or Ready messages
        if (state.stage != MessageStage.Pending && state.stage != MessageStage.Ready) {
            revert CannotExpire(messageId, state.stage);
        }

        if (!_isExpired(messageId)) {
            ApplicationConfig storage config = _applicationConfigs[state.domain];
            revert NotYetExpired(messageId, state.timestamp + config.expirationSeconds);
        }

        state.stage = MessageStage.Expired;
        emit MessageExpired(messageId, block.timestamp);
    }

    function _isExpired(bytes32 messageId) internal view returns (bool) {
        MessageStateV2 storage state = _messageStates[messageId];
        if (state.stage == MessageStage.NotInitialized) return false;
        if (state.stage >= MessageStage.Completed) return false; // Already terminal

        ApplicationConfig storage config = _applicationConfigs[state.domain];
        uint64 expirationTime = state.timestamp + config.expirationSeconds;
        return block.timestamp > expirationTime;
    }

    // ============================================================
    // MESSAGE LIFECYCLE - REJECTION
    // ============================================================

    /// @inheritdoc IMessageBridge
    function rejectMessage(bytes32 messageId, bytes32 reasonHash, string calldata reasonRef) external {
        MessageStateV2 storage state = _messageStates[messageId];

        // Must be initialized
        if (state.stage == MessageStage.NotInitialized) {
            revert MessageNotInitialized(messageId);
        }

        // Caller must be a validator
        if (!_validatorInfo[msg.sender].active) {
            revert ValidatorNotAuthorized(msg.sender);
        }

        // Cannot reject twice
        if (_hasValidatorRejected[messageId][msg.sender]) {
            revert AlreadyRejected(messageId, msg.sender);
        }

        // Record rejection
        _hasValidatorRejected[messageId][msg.sender] = true;
        state.rejectionsCollected++;

        _messageRejections[messageId].push(
            Rejection({validator: msg.sender, reasonHash: reasonHash, reasonRef: reasonRef, timestamp: uint64(block.timestamp)})
        );

        emit MessageRejected(messageId, msg.sender, reasonHash);
    }

    /// @inheritdoc IMessageBridge
    function rejectProposal(
        bytes32 messageId,
        string calldata messageType,
        bytes32 domain,
        uint64 nonce,
        bytes32 reasonHash,
        string calldata reasonRef
    ) external {
        // Must be Primary Validator for domain
        ApplicationConfig storage config = _applicationConfigs[domain];
        if (config.primaryValidator != msg.sender) {
            revert NotPrimaryValidator(domain, msg.sender);
        }

        // Message must not be initialized
        if (_messageStates[messageId].stage != MessageStage.NotInitialized) {
            revert MessageAlreadyInitialized(messageId);
        }

        // Proposal must not already be rejected
        if (proposalRejected[messageId]) {
            revert ProposalAlreadyRejected(messageId);
        }

        // Consume the nonce
        _lastNonces.validateAndConsume(domain, nonce);

        // Mark proposal as rejected
        proposalRejected[messageId] = true;

        emit ProposalRejected(messageId, domain, nonce, msg.sender, reasonHash);
    }

    // ============================================================
    // BATCH OPERATIONS
    // ============================================================

    /// @inheritdoc IMessageBridge
    function batchSignMessages(bytes32[] calldata messageIds, bytes[] calldata signatures) external whenNotPaused {
        if (messageIds.length != signatures.length) {
            revert ArrayLengthMismatch(messageIds.length, signatures.length);
        }
        if (messageIds.length > MAX_BATCH_SIZE) {
            revert BatchSizeExceeded(messageIds.length, MAX_BATCH_SIZE);
        }

        for (uint256 i = 0; i < messageIds.length; i++) {
            // Skip if not pending or would revert
            MessageStateV2 storage state = _messageStates[messageIds[i]];
            if (state.stage == MessageStage.Pending && !_isExpired(messageIds[i])) {
                // Try to sign, but don't revert batch on individual failure
                try this.signMessageInternal(messageIds[i], signatures[i]) {} catch {}
            }
        }
    }

    /// @dev Internal function for batch signing (allows try/catch)
    function signMessageInternal(bytes32 messageId, bytes calldata signature) external {
        require(msg.sender == address(this), "Only self");
        _signMessage(messageId, signature);
    }

    /// @inheritdoc IMessageBridge
    function batchExecuteMessages(bytes32[] calldata messageIds) external nonReentrant whenNotPaused {
        if (messageIds.length > MAX_BATCH_SIZE) {
            revert BatchSizeExceeded(messageIds.length, MAX_BATCH_SIZE);
        }

        for (uint256 i = 0; i < messageIds.length; i++) {
            // Skip if not ready or would revert
            MessageStateV2 storage state = _messageStates[messageIds[i]];
            if (state.stage == MessageStage.Ready && !_isExpired(messageIds[i])) {
                // Execute (will update state internally)
                try this.executeMessageInternal(messageIds[i]) {} catch {}
            }
        }
    }

    /// @dev Internal function for batch execution (allows try/catch)
    function executeMessageInternal(bytes32 messageId) external nonReentrant {
        require(msg.sender == address(this), "Only self");
        // Inline execution logic to avoid reentrancy issues
        MessageStateV2 storage state = _messageStates[messageId];
        if (state.stage != MessageStage.Ready) return;

        MessageTypeConfig storage typeConfig = _messageTypes[state.messageType];
        address target = typeConfig.target;

        state.stage = MessageStage.PreExecution;
        state.stage = MessageStage.Executing;

        if (state.value > 0) {
            weth.withdraw(state.value);
        }

        (bool success,) = target.call{value: state.value}(state.calldata_);

        state.stage = MessageStage.PostExecution;
        state.stage = success ? MessageStage.Completed : MessageStage.Failed;
    }

    // ============================================================
    // QUERY FUNCTIONS - Messages
    // ============================================================

    /// @inheritdoc IMessageBridge
    function getMessageState(bytes32 messageId) external view returns (MessageStateV2 memory) {
        return _messageStates[messageId];
    }

    /// @inheritdoc IMessageBridge
    function getMessageStage(bytes32 messageId) external view returns (MessageStage) {
        return _messageStates[messageId].stage;
    }

    /// @inheritdoc IMessageBridge
    function getSignatureCount(bytes32 messageId) external view returns (uint256) {
        return _messageStates[messageId].signaturesCollected;
    }

    /// @inheritdoc IMessageBridge
    function getRejectionCount(bytes32 messageId) external view returns (uint256) {
        return _messageStates[messageId].rejectionsCollected;
    }

    /// @inheritdoc IMessageBridge
    function isExpired(bytes32 messageId) external view returns (bool) {
        return _isExpired(messageId);
    }

    /// @inheritdoc IMessageBridge
    function hasValidatorRejected(bytes32 messageId, address validator) external view returns (bool) {
        return _hasValidatorRejected[messageId][validator];
    }

    /// @inheritdoc IMessageBridge
    function computeMessageId(
        string calldata messageType,
        bytes calldata calldata_,
        bytes32 metadataHash,
        uint64 nonce,
        uint64 timestamp,
        bytes32 domain
    ) public pure returns (bytes32) {
        return keccak256(abi.encode(messageType, keccak256(calldata_), metadataHash, nonce, timestamp, domain));
    }

    // ============================================================
    // MESSAGE TYPE REGISTRY
    // ============================================================

    /// @inheritdoc IMessageTypeRegistry
    function registerMessageType(
        string calldata messageType,
        address target,
        bytes32 schemaHash,
        string calldata schemaUri
    ) external onlyRole(REGISTRY_ROLE) {
        if (_messageTypes[messageType].target != address(0)) {
            revert MessageTypeAlreadyRegistered(messageType);
        }
        if (target == address(0)) revert InvalidAddress("target");

        bytes4 selector = bytes4(keccak256(bytes(messageType)));

        _messageTypes[messageType] = MessageTypeConfig({
            selector: selector,
            target: target,
            schemaHash: schemaHash,
            schemaUri: schemaUri,
            enabled: true,
            updatedAt: uint64(block.timestamp)
        });

        _selectorToMessageType[selector] = messageType;

        emit MessageTypeRegistered(messageType, selector, target, schemaHash, schemaUri);
    }

    /// @inheritdoc IMessageTypeRegistry
    function updateSchema(string calldata messageType, bytes32 newSchemaHash, string calldata newSchemaUri)
        external
        onlyRole(REGISTRY_ROLE)
    {
        MessageTypeConfig storage config = _messageTypes[messageType];
        if (config.target == address(0)) revert MessageTypeNotRegistered(messageType);

        bytes32 oldSchemaHash = config.schemaHash;
        config.schemaHash = newSchemaHash;
        config.schemaUri = newSchemaUri;
        config.updatedAt = uint64(block.timestamp);

        emit SchemaUpdated(messageType, oldSchemaHash, newSchemaHash, newSchemaUri);
    }

    /// @inheritdoc IMessageTypeRegistry
    function setEnabled(string calldata messageType, bool enabled) external onlyRole(REGISTRY_ROLE) {
        MessageTypeConfig storage config = _messageTypes[messageType];
        if (config.target == address(0)) revert MessageTypeNotRegistered(messageType);

        config.enabled = enabled;
        config.updatedAt = uint64(block.timestamp);

        emit MessageTypeEnabled(messageType, enabled);
    }

    /// @inheritdoc IMessageTypeRegistry
    function setTarget(string calldata messageType, address newTarget) external onlyRole(REGISTRY_ROLE) {
        MessageTypeConfig storage config = _messageTypes[messageType];
        if (config.target == address(0)) revert MessageTypeNotRegistered(messageType);
        if (newTarget == address(0)) revert InvalidAddress("target");

        address oldTarget = config.target;
        config.target = newTarget;
        config.updatedAt = uint64(block.timestamp);

        emit TargetUpdated(messageType, oldTarget, newTarget);
    }

    /// @inheritdoc IMessageTypeRegistry
    function getMessageTypeConfig(string calldata messageType) external view returns (MessageTypeConfig memory) {
        return _messageTypes[messageType];
    }

    /// @inheritdoc IMessageTypeRegistry
    function isMessageTypeRegistered(string calldata messageType) external view returns (bool) {
        return _messageTypes[messageType].target != address(0);
    }

    /// @inheritdoc IMessageTypeRegistry
    function isMessageTypeEnabled(string calldata messageType) external view returns (bool) {
        return _messageTypes[messageType].enabled;
    }

    /// @inheritdoc IMessageTypeRegistry
    function getMessageTypeBySelector(bytes4 selector) external view returns (string memory) {
        return _selectorToMessageType[selector];
    }

    /// @inheritdoc IMessageTypeRegistry
    function getTarget(string calldata messageType) external view returns (address) {
        return _messageTypes[messageType].target;
    }

    /// @inheritdoc IMessageTypeRegistry
    function getSchemaHash(string calldata messageType) external view returns (bytes32) {
        return _messageTypes[messageType].schemaHash;
    }

    // ============================================================
    // APPLICATION REGISTRY
    // ============================================================

    /// @inheritdoc IApplicationRegistry
    function registerApplication(bytes32 domain, ApplicationConfig calldata config) external onlyRole(REGISTRY_ROLE) {
        if (_applicationConfigs[domain].primaryValidator != address(0)) {
            revert ApplicationAlreadyRegistered(domain);
        }
        if (config.primaryValidator == address(0)) revert InvalidAddress("primaryValidator");

        // Ensure primary validator is registered
        if (!_validatorInfo[config.primaryValidator].active) {
            revert ValidatorNotAuthorized(config.primaryValidator);
        }

        _applicationConfigs[domain] = config;
        _lastNonces[domain] = 0;

        emit ApplicationRegistered(domain, config.primaryValidator, config.expirationSeconds, config.requireWitnessSignatures);
    }

    /// @inheritdoc IApplicationRegistry
    function updateApplicationConfig(bytes32 domain, ApplicationConfig calldata config)
        external
        onlyRole(REGISTRY_ROLE)
    {
        if (_applicationConfigs[domain].primaryValidator == address(0)) {
            revert ApplicationNotRegistered(domain);
        }
        if (config.primaryValidator == address(0)) revert InvalidAddress("primaryValidator");

        _applicationConfigs[domain] = config;

        emit ApplicationConfigUpdated(domain, config.primaryValidator);
    }

    /// @inheritdoc IApplicationRegistry
    function setApplicationActive(bytes32 domain, bool active) external onlyRole(REGISTRY_ROLE) {
        if (_applicationConfigs[domain].primaryValidator == address(0)) {
            revert ApplicationNotRegistered(domain);
        }

        _applicationConfigs[domain].active = active;

        emit ApplicationActiveChanged(domain, active);
    }

    /// @inheritdoc IApplicationRegistry
    function setPrimaryValidator(bytes32 domain, address validator, bytes calldata) external onlyRole(VALIDATOR_MANAGER_ROLE) {
        if (_applicationConfigs[domain].primaryValidator == address(0)) {
            revert ApplicationNotRegistered(domain);
        }
        if (validator == address(0)) revert InvalidAddress("validator");

        address oldValidator = _applicationConfigs[domain].primaryValidator;
        _applicationConfigs[domain].primaryValidator = validator;

        // Register validator if not already
        if (!_validatorInfo[validator].active) {
            _validatorInfo[validator] = ValidatorInfo({
                validator: validator,
                active: true,
                isWitness: false,
                registeredAt: uint64(block.timestamp),
                teeCodeHash: bytes32(0),
                teeVersion: "",
                lastAttestationAt: 0
            });
        }

        emit PrimaryValidatorSet(domain, oldValidator, validator);
    }

    /// @inheritdoc IApplicationRegistry
    function addWitnessValidator(address validator, bytes calldata) external onlyRole(VALIDATOR_MANAGER_ROLE) {
        if (validator == address(0)) revert InvalidAddress("validator");

        _validatorInfo[validator] = ValidatorInfo({
            validator: validator,
            active: true,
            isWitness: true,
            registeredAt: uint64(block.timestamp),
            teeCodeHash: bytes32(0),
            teeVersion: "",
            lastAttestationAt: 0
        });

        _witnessValidators.add(validator);

        emit WitnessValidatorAdded(validator, bytes32(0), "");
    }

    /// @inheritdoc IApplicationRegistry
    function removeValidator(address validator) external onlyRole(VALIDATOR_MANAGER_ROLE) {
        if (validator == address(0)) revert InvalidAddress("validator");

        _validatorInfo[validator].active = false;
        _witnessValidators.remove(validator);

        emit ValidatorRemoved(validator);
    }

    /// @inheritdoc IApplicationRegistry
    function updateValidatorAttestation(address validator, bytes calldata, string calldata version)
        external
        onlyRole(VALIDATOR_MANAGER_ROLE)
    {
        if (!_validatorInfo[validator].active) {
            revert ValidatorNotAuthorized(validator);
        }

        _validatorInfo[validator].teeVersion = version;
        _validatorInfo[validator].lastAttestationAt = uint64(block.timestamp);

        emit ValidatorAttestationUpdated(validator, bytes32(0), version);
    }

    /// @inheritdoc IApplicationRegistry
    function setSignatureThreshold(uint256 threshold) external onlyRole(DEFAULT_ADMIN_ROLE) {
        if (threshold < MIN_THRESHOLD) {
            revert InvalidThreshold(threshold, _witnessValidators.length());
        }

        uint256 oldThreshold = signatureThreshold;
        signatureThreshold = threshold;

        emit ThresholdUpdated(oldThreshold, threshold);
    }

    /// @inheritdoc IApplicationRegistry
    function setMinimumTeeVersion(string calldata version) external onlyRole(DEFAULT_ADMIN_ROLE) {
        string memory oldVersion = minimumTeeVersion;
        minimumTeeVersion = version;

        emit MinimumTeeVersionUpdated(oldVersion, version);
    }

    /// @inheritdoc IApplicationRegistry
    function getApplicationConfig(bytes32 domain) external view returns (ApplicationConfig memory) {
        return _applicationConfigs[domain];
    }

    /// @inheritdoc IApplicationRegistry
    function isApplicationRegistered(bytes32 domain) external view returns (bool) {
        return _applicationConfigs[domain].primaryValidator != address(0);
    }

    /// @inheritdoc IApplicationRegistry
    function isApplicationActive(bytes32 domain) external view returns (bool) {
        return _applicationConfigs[domain].active;
    }

    /// @inheritdoc IApplicationRegistry
    function getLastNonce(bytes32 domain) external view returns (uint64) {
        return _lastNonces[domain];
    }

    /// @inheritdoc IApplicationRegistry
    function getValidatorInfo(address validator) external view returns (ValidatorInfo memory) {
        return _validatorInfo[validator];
    }

    /// @inheritdoc IApplicationRegistry
    function isActiveValidator(address validator) external view returns (bool) {
        return _validatorInfo[validator].active;
    }

    /// @inheritdoc IApplicationRegistry
    function isWitnessValidator(address validator) external view returns (bool) {
        return _validatorInfo[validator].isWitness && _validatorInfo[validator].active;
    }

    /// @inheritdoc IApplicationRegistry
    function getWitnessValidators() external view returns (address[] memory) {
        return _witnessValidators.values();
    }

    /// @inheritdoc IApplicationRegistry
    function getActiveValidatorCount() external view returns (uint256) {
        return _witnessValidators.length();
    }

    /// @inheritdoc IApplicationRegistry
    function getSignatureThreshold() external view returns (uint256) {
        return signatureThreshold;
    }

    /// @inheritdoc IApplicationRegistry
    function getMinimumTeeVersion() external view returns (string memory) {
        return minimumTeeVersion;
    }

    // ============================================================
    // EMERGENCY FUNCTIONS
    // ============================================================

    /**
     * @notice Pause the contract
     * @dev Only callable by admin. Blocks initialize, sign, execute.
     */
    function pause() external onlyRole(DEFAULT_ADMIN_ROLE) {
        _pause();
    }

    /**
     * @notice Unpause the contract
     * @dev Only callable by admin
     */
    function unpause() external onlyRole(DEFAULT_ADMIN_ROLE) {
        _unpause();
    }

    /**
     * @notice Emergency withdraw WETH
     * @dev Only callable by admin when paused
     * @param recipient Address to receive WETH
     * @param amount Amount of WETH to withdraw
     */
    function emergencyWithdrawWETH(address recipient, uint256 amount) external onlyRole(DEFAULT_ADMIN_ROLE) {
        require(paused(), "Not paused");
        if (recipient == address(0)) revert InvalidAddress("recipient");

        weth.transfer(recipient, amount);
    }
}
