// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {MessageStage} from "src/types/DataTypes.sol";

/**
 * @title Errors
 * @notice Custom error definitions for the Message Passing Bridge
 * @dev All errors are grouped by category for easier navigation
 */

// ============================================================
// AUTHORIZATION ERRORS
// ============================================================

/// @notice Caller is not the Primary Validator for this domain
error NotPrimaryValidator(bytes32 domain, address caller);

/// @notice Caller is not a registered validator
error NotValidator(address caller);

/// @notice Caller does not have the required role
error NotAuthorized(address caller, bytes32 role);

/// @notice Validator is not authorized (not active or not registered)
error ValidatorNotAuthorized(address validator);

// ============================================================
// MESSAGE STATE ERRORS
// ============================================================

/// @notice Message has not been initialized
error MessageNotInitialized(bytes32 messageId);

/// @notice Message has already been initialized
error MessageAlreadyInitialized(bytes32 messageId);

/// @notice Message is not in Pending stage
error MessageNotPending(bytes32 messageId, MessageStage currentStage);

/// @notice Message is not in Ready stage
error MessageNotReady(bytes32 messageId, MessageStage currentStage);

/// @notice Message is already in a terminal state
error MessageAlreadyTerminal(bytes32 messageId, MessageStage currentStage);

/// @notice Message cannot transition to the requested state
error InvalidStateTransition(bytes32 messageId, MessageStage from, MessageStage to);

// ============================================================
// NONCE & EXPIRATION ERRORS
// ============================================================

/// @notice Nonce is not the expected next value
error InvalidNonce(bytes32 domain, uint64 expected, uint64 provided);

/// @notice Message has expired
error MessageIsExpired(bytes32 messageId, uint256 expiredAt);

/// @notice Message cannot be expired (not in valid state)
error CannotExpire(bytes32 messageId, MessageStage currentStage);

/// @notice Message has not yet expired
error NotYetExpired(bytes32 messageId, uint256 expiresAt);

/// @notice Timestamp is outside acceptable range
error TimestampOutOfRange(uint64 timestamp, uint256 currentTime, uint256 maxDrift);

// ============================================================
// SIGNATURE & REJECTION ERRORS
// ============================================================

/// @notice Validator has already signed this message
error AlreadySigned(bytes32 messageId, address validator);

/// @notice Validator has already rejected this message
error AlreadyRejected(bytes32 messageId, address validator);

/// @notice Proposal has already been rejected
error ProposalAlreadyRejected(bytes32 messageId);

/// @notice Signature is invalid or from unknown signer
error InvalidSignature(bytes32 messageId, address recoveredSigner);

/// @notice Message ID does not match computed value
error InvalidMessageId(bytes32 provided, bytes32 computed);

// ============================================================
// EXECUTION ERRORS
// ============================================================

/// @notice Target contract call failed
error ExecutionFailed(bytes32 messageId, bytes returnData);

/// @notice Module check failed during pre or post execution
error ModuleCheckFailed(bytes32 messageId, address module, string reason);

/// @notice Insufficient WETH balance for execution
error InsufficientWETHBalance(uint256 required, uint256 available);

/// @notice Re-entrancy detected during execution
error ReentrancyDetected(bytes32 messageId);

// ============================================================
// CONFIGURATION ERRORS
// ============================================================

/// @notice Message type is not registered
error MessageTypeNotRegistered(string messageType);

/// @notice Message type is disabled
error MessageTypeDisabled(string messageType);

/// @notice Message type already registered
error MessageTypeAlreadyRegistered(string messageType);

/// @notice Application domain is not registered
error ApplicationNotRegistered(bytes32 domain);

/// @notice Application domain is not active
error ApplicationNotActive(bytes32 domain);

/// @notice Application domain already registered
error ApplicationAlreadyRegistered(bytes32 domain);

/// @notice Signature threshold is invalid
error InvalidThreshold(uint256 threshold, uint256 validatorCount);

/// @notice Threshold not yet reached
error ThresholdNotReached(bytes32 messageId, uint256 current, uint256 required);

// ============================================================
// VALIDATION ERRORS
// ============================================================

/// @notice Address parameter is invalid (zero address)
error InvalidAddress(string param);

/// @notice TEE attestation is invalid
error InvalidAttestation(address validator);

/// @notice TEE version is below minimum required
error VersionBelowMinimum(string provided, string minimum);

/// @notice Batch size exceeds maximum allowed
error BatchSizeExceeded(uint256 provided, uint256 maximum);

/// @notice Storage reference URI is too long
error StorageRefTooLong(uint256 provided, uint256 maximum);

/// @notice Array length mismatch in batch operation
error ArrayLengthMismatch(uint256 expected, uint256 provided);

// ============================================================
// UPGRADE & EMERGENCY ERRORS
// ============================================================

/// @notice Contract is paused
error ContractPaused();

/// @notice Contract is not paused
error ContractNotPaused();

/// @notice Upgrade timelock has not elapsed
error UpgradeTimelockNotElapsed(uint256 unlockTime, uint256 currentTime);

/// @notice No pending upgrade
error NoPendingUpgrade();

/// @notice Upgrade implementation mismatch
error UpgradeImplementationMismatch(address expected, address provided);
