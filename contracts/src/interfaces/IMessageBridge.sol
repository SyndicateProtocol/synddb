// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {MessageStage, MessageStateV2, ApplicationConfig, ValidatorInfo, Rejection} from "src/types/DataTypes.sol";

/**
 * @title IMessageBridge
 * @notice Interface for the Message Passing Bridge contract
 * @dev Defines the core message lifecycle, validator management, and query functions
 */
interface IMessageBridge {
    // ============================================================
    // EVENTS
    // ============================================================

    /// @notice Emitted when a message is initialized by a Primary Validator
    event MessageInitialized(
        bytes32 indexed messageId,
        bytes32 indexed domain,
        address indexed primaryValidator,
        string messageType,
        string storageRef
    );

    /// @notice Emitted when a validator submits a signature
    event SignatureSubmitted(bytes32 indexed messageId, address indexed validator, uint256 signaturesCollected);

    /// @notice Emitted when the signature threshold is reached
    event ThresholdReached(bytes32 indexed messageId, uint256 signaturesCollected);

    /// @notice Emitted when a message is successfully executed
    event MessageExecuted(bytes32 indexed messageId, address indexed target, bool success);

    /// @notice Emitted when message execution fails
    event MessageFailed(bytes32 indexed messageId, bytes returnData);

    /// @notice Emitted when a message expires
    event MessageExpired(bytes32 indexed messageId, uint256 expiredAt);

    /// @notice Emitted when a validator rejects an initialized message
    event MessageRejected(bytes32 indexed messageId, address indexed validator, bytes32 reasonHash);

    /// @notice Emitted when a Primary Validator rejects a proposal before initialization
    event ProposalRejected(
        bytes32 indexed messageId, bytes32 indexed domain, uint64 nonce, address indexed validator, bytes32 reasonHash
    );

    // ============================================================
    // MESSAGE LIFECYCLE
    // ============================================================

    /**
     * @notice Initialize a new message
     * @dev Only callable by the Primary Validator for the specified domain
     * @param messageId Pre-computed message ID
     * @param messageType ABI function signature (e.g., "mint(address,uint256)")
     * @param calldata_ ABI-encoded function parameters
     * @param metadataHash keccak256 of canonical metadata JSON
     * @param storageRef URI(s) to full metadata (ar://...|ipfs://...)
     * @param nonce Per-domain sequential nonce
     * @param timestamp Message creation timestamp
     * @param domain Application identifier
     */
    function initializeMessage(
        bytes32 messageId,
        string calldata messageType,
        bytes calldata calldata_,
        bytes32 metadataHash,
        string calldata storageRef,
        uint64 nonce,
        uint64 timestamp,
        bytes32 domain
    ) external payable;

    /**
     * @notice Initialize a message and submit Primary Validator's signature
     * @dev Convenience function combining initializeMessage and signMessage
     */
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
    ) external payable;

    /**
     * @notice Submit a validator signature for a pending message
     * @dev Signatures are verified using EIP-712 typed data
     * @param messageId The message to sign
     * @param signature ECDSA signature over EIP-712 digest
     */
    function signMessage(bytes32 messageId, bytes calldata signature) external;

    /**
     * @notice Execute a message that has reached the signature threshold
     * @dev Runs pre-execution modules, calls target, then post-execution modules
     * @param messageId The message to execute
     */
    function executeMessage(bytes32 messageId) external;

    /**
     * @notice Mark an expired message as Expired terminal state
     * @dev Anyone can call this if the message has passed its expiration time
     * @param messageId The message to expire
     */
    function expireMessage(bytes32 messageId) external;

    /**
     * @notice Reject an initialized message (Witness Validator action)
     * @dev Records rejection but does not prevent execution if threshold is met
     * @param messageId The message to reject
     * @param reasonHash keccak256 of rejection reason JSON
     * @param reasonRef Storage reference to full reason
     */
    function rejectMessage(bytes32 messageId, bytes32 reasonHash, string calldata reasonRef) external;

    /**
     * @notice Reject a message proposal before initialization (Primary Validator action)
     * @dev Consumes the nonce to prevent the message from being initialized
     * @param messageId Pre-computed message ID
     * @param messageType The message type being rejected
     * @param domain Application domain
     * @param nonce The nonce to consume
     * @param reasonHash keccak256 of rejection reason JSON
     * @param reasonRef Storage reference to full reason
     */
    function rejectProposal(
        bytes32 messageId,
        string calldata messageType,
        bytes32 domain,
        uint64 nonce,
        bytes32 reasonHash,
        string calldata reasonRef
    ) external;

    // ============================================================
    // BATCH OPERATIONS
    // ============================================================

    /**
     * @notice Submit multiple signatures in a single transaction
     * @dev Skips messages that are not pending or already signed by the validator
     * @param messageIds Array of message IDs
     * @param signatures Array of corresponding signatures
     */
    function batchSignMessages(bytes32[] calldata messageIds, bytes[] calldata signatures) external;

    /**
     * @notice Execute multiple ready messages in a single transaction
     * @dev Skips messages that are not ready or already executed
     * @param messageIds Array of message IDs to execute
     */
    function batchExecuteMessages(bytes32[] calldata messageIds) external;

    // ============================================================
    // QUERY FUNCTIONS
    // ============================================================

    /**
     * @notice Get the full state of a message
     * @param messageId The message to query
     * @return state The complete message state
     */
    function getMessageState(bytes32 messageId) external view returns (MessageStateV2 memory state);

    /**
     * @notice Get the current stage of a message
     * @param messageId The message to query
     * @return stage The current lifecycle stage
     */
    function getMessageStage(bytes32 messageId) external view returns (MessageStage stage);

    /**
     * @notice Get the number of signatures collected for a message
     * @param messageId The message to query
     * @return count The number of valid signatures
     */
    function getSignatureCount(bytes32 messageId) external view returns (uint256 count);

    /**
     * @notice Get the number of rejections for a message
     * @param messageId The message to query
     * @return count The number of rejections
     */
    function getRejectionCount(bytes32 messageId) external view returns (uint256 count);

    /**
     * @notice Check if a message has expired
     * @param messageId The message to check
     * @return expired True if the message has passed its expiration time
     */
    function isExpired(bytes32 messageId) external view returns (bool expired);

    /**
     * @notice Check if a validator has signed a message
     * @param messageId The message to check
     * @param validator The validator address
     * @return signed True if the validator has signed
     */
    function hasValidatorSigned(bytes32 messageId, address validator) external view returns (bool signed);

    /**
     * @notice Check if a validator has rejected a message
     * @param messageId The message to check
     * @param validator The validator address
     * @return rejected True if the validator has rejected
     */
    function hasValidatorRejected(bytes32 messageId, address validator) external view returns (bool rejected);

    /**
     * @notice Compute the message ID from message parameters
     * @dev Deterministic: same inputs always produce same messageId
     */
    function computeMessageId(
        string calldata messageType,
        bytes calldata calldata_,
        bytes32 metadataHash,
        uint64 nonce,
        uint64 timestamp,
        bytes32 domain
    ) external pure returns (bytes32 messageId);

    // ============================================================
    // CONFIGURATION
    // ============================================================

    /**
     * @notice Get the current signature threshold
     * @return threshold The minimum number of signatures required
     */
    function signatureThreshold() external view returns (uint256 threshold);

    /**
     * @notice Get the EIP-712 domain separator
     * @return separator The domain separator bytes32
     */
    function DOMAIN_SEPARATOR() external view returns (bytes32 separator);
}
