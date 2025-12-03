// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

/**
 * @title IValidatorSigningAndQuery
 * @notice Interface for validators to sign messages and query signature counts
 * @dev Supports both direct validator signing and relayer-based signature submission
 */
interface IValidatorSigningAndQuery {
    /**
     * @notice Emitted when a validator signs a message
     * @dev This event is emitted by both signMessage() (when validator signs directly) and
     *      signMessageWithSignature() (when a relayer submits a signature on behalf of a validator)
     * @param messageId The unique identifier of the signed message
     * @param validator The address of the validator who signed
     * @param submittedBy The address that submitted the signature (msg.sender). Useful for tracking relayer activity and rewards.
     */
    event MessageSigned(bytes32 indexed messageId, address indexed validator, address indexed submittedBy);

    /**
     * @notice Thrown when a signature comes from an unauthorized validator
     */
    error ValidatorNotAuthorized();

    /**
     * @notice Allows a validator to directly sign a message
     * @dev Uses msg.sender as the validator address (direct call pattern)
     * @param messageId The unique identifier of the message to sign
     */
    function signMessage(bytes32 messageId) external;

    /**
     * @notice Allows anyone to submit a validator's signature for a message
     * @dev Verifies the signature cryptographically (relayer pattern)
     * @param messageId The unique identifier of the message to sign
     * @param signature The ECDSA signature from an authorized validator
     */
    function signMessageWithSignature(bytes32 messageId, bytes calldata signature) external;

    /**
     * @notice Counts how many of the provided validators have signed a specific message
     * @param messageId The unique identifier of the message
     * @param validators Array of validator addresses to check
     * @return uint256 The number of validators who have signed the message
     */
    function getValidatorSignatureCount(bytes32 messageId, address[] calldata validators)
        external
        view
        returns (uint256);
}
