// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {IModuleCheck} from "src/interfaces/IModuleCheck.sol";
import {ProcessingStage, SequencerSignature} from "src/types/DataTypes.sol";
import {Ownable} from "@openzeppelin/contracts/access/Ownable.sol";

/**
 * @title MessageOrderingModule
 * @notice Validation module that enforces sequential message processing based on nonces
 * @dev Ensures messages from a specific sender (or chain/sender) are processed in order.
 *      This prevents out-of-order execution which could lead to race conditions or state inconsistencies.
 *
 *      Example use case: A cross-chain DEX needs to ensure swap orders are executed in the order
 *      they were submitted to prevent front-running or incorrect price execution.
 */
contract MessageOrderingModule is IModuleCheck, Ownable {
    /**
     * @notice Stores the expected next nonce for each tracked identifier
     * @dev The identifier can be:
     *      - A single address (e.g., sender address)
     *      - A composite key (e.g., keccak256(abi.encodePacked(chainId, sender)))
     */
    mapping(bytes32 identifier => uint256 expectedNonce) public expectedNonces;

    /**
     * @notice Emitted when a message's nonce is validated and the expected nonce is incremented
     * @param identifier The tracked identifier (sender or composite key)
     * @param nonce The nonce that was successfully validated
     * @param nextExpectedNonce The next nonce expected for this identifier
     */
    event NonceValidated(bytes32 indexed identifier, uint256 nonce, uint256 nextExpectedNonce);

    /**
     * @notice Emitted when the expected nonce for an identifier is reset by the owner
     * @param identifier The tracked identifier
     * @param newNonce The new expected nonce value
     */
    event NonceReset(bytes32 indexed identifier, uint256 newNonce);

    error InvalidNonce(bytes32 identifier, uint256 expected, uint256 actual);
    error InvalidMessageIdFormat();

    /**
     * @notice Initializes the module with an owner address
     * @param _owner Address that can reset nonces if needed
     */
    constructor(address _owner) Ownable(_owner) {}

    /**
     * @notice Validates message ordering based on embedded nonce
     * @dev Only validates during PreExecution stage. The payload must contain the ordering metadata
     *      encoded as the first 64 bytes: abi.encodePacked(identifier, nonce)
     *
     *      The identifier (bytes32) can be:
     *      - Single address: bytes32(uint256(uint160(sender)))
     *      - Composite: keccak256(abi.encodePacked(sourceChainId, sender))
     *
     *      The nonce (uint256) follows the identifier in the payload.
     *
     *      If the payload is less than 64 bytes, ordering is not enforced and the check passes.
     *      This allows optional ordering enforcement per message.
     * @param messageId Unique identifier of the message
     * @param stage Current processing stage (only validates during PreExecution)
     * @param payload Message payload containing ordering metadata in first 64 bytes
     * @return bool True if nonce is valid and sequential, or if ordering is not enforced
     */
    function check(bytes32 messageId, ProcessingStage stage, bytes memory payload, SequencerSignature memory)
        external
        returns (bool)
    {
        // Only validate during PreExecution to ensure ordering before execution
        if (stage != ProcessingStage.PreExecution) {
            return true;
        }

        // If payload is too short, skip ordering validation (optional enforcement)
        if (payload.length < 64) {
            return true;
        }

        // Extract identifier (first 32 bytes) and nonce (next 32 bytes) from payload
        bytes32 identifier;
        uint256 nonce;

        assembly {
            // Load identifier from payload offset 32 (skip length prefix)
            identifier := mload(add(payload, 32))
            // Load nonce from payload offset 64
            nonce := mload(add(payload, 64))
        }

        // Validate the nonce for this identifier
        return validateNonce(identifier, nonce);
    }

    /**
     * @notice Validates a message with explicit identifier and nonce
     * @dev This is a helper function for explicit nonce validation when the identifier
     *      and nonce are known separately from the messageId.
     * @param identifier The tracking identifier (sender address or composite key)
     * @param nonce The nonce to validate
     * @return bool True if the nonce is valid and sequential
     */
    function validateNonce(bytes32 identifier, uint256 nonce) public returns (bool) {
        uint256 expected = expectedNonces[identifier];

        if (nonce != expected) {
            revert InvalidNonce(identifier, expected, nonce);
        }

        // Increment expected nonce for next message
        expectedNonces[identifier] = expected + 1;

        emit NonceValidated(identifier, nonce, expected + 1);

        return true;
    }

    /**
     * @notice Gets the expected nonce for an identifier
     * @param identifier The tracking identifier
     * @return uint256 The next expected nonce for this identifier
     */
    function getExpectedNonce(bytes32 identifier) external view returns (uint256) {
        return expectedNonces[identifier];
    }

    /**
     * @notice Resets the expected nonce for an identifier (emergency function)
     * @dev Only callable by owner. Use with caution as this can break ordering guarantees.
     *      Should only be used in emergency situations or during system migrations.
     * @param identifier The tracking identifier to reset
     * @param newNonce The new expected nonce value
     */
    function resetNonce(bytes32 identifier, uint256 newNonce) external onlyOwner {
        expectedNonces[identifier] = newNonce;
        emit NonceReset(identifier, newNonce);
    }

    /**
     * @notice Batch reset nonces for multiple identifiers
     * @dev Only callable by owner. Useful for system migrations or batch corrections.
     * @param identifiers Array of tracking identifiers to reset
     * @param newNonces Array of new expected nonce values (must match length of identifiers)
     */
    function batchResetNonces(bytes32[] calldata identifiers, uint256[] calldata newNonces) external onlyOwner {
        require(identifiers.length == newNonces.length, "Array length mismatch");

        for (uint256 i = 0; i < identifiers.length; i++) {
            expectedNonces[identifiers[i]] = newNonces[i];
            emit NonceReset(identifiers[i], newNonces[i]);
        }
    }
}
