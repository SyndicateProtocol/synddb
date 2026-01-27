// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {IAttestationVerifier} from "src/interfaces/IAttestationVerifier.sol";

/**
 * @title ITeeKeyManager
 * @notice Interface for managing TEE signing keys for sequencers and validators
 * @dev Validates and tracks signing keys from trusted execution environments.
 *      Supports separate key sets for sequencers and validators with optional expiration.
 */
interface ITeeKeyManager {
    /*//////////////////////////////////////////////////////////////
                            KEY VALIDATION
    //////////////////////////////////////////////////////////////*/

    /**
     * @notice Checks if a sequencer key is valid (registered and not expired)
     * @param publicKey The address to check
     * @return True if the key is valid, reverts otherwise
     */
    function isSequencerKeyValid(address publicKey) external view returns (bool);

    /**
     * @notice Checks if a validator key is valid (registered and not expired)
     * @param publicKey The address to check
     * @return True if the key is valid, reverts otherwise
     */
    function isValidatorKeyValid(address publicKey) external view returns (bool);

    /*//////////////////////////////////////////////////////////////
                            KEY REGISTRATION
    //////////////////////////////////////////////////////////////*/

    /**
     * @notice Registers a new sequencer key with an attestation proof
     * @param publicValues The encoded public values from the attestation
     * @param proofBytes The SP1 proof bytes
     * @param requiresApproval Whether the key needs owner approval
     * @param expiresAt Expiration timestamp (0 = never expires)
     * @return publicKey The registered key address
     */
    function addSequencerKey(
        bytes calldata publicValues,
        bytes calldata proofBytes,
        bool requiresApproval,
        uint256 expiresAt
    ) external returns (address publicKey);

    /**
     * @notice Registers a new validator key with an attestation proof
     * @param publicValues The encoded public values from the attestation
     * @param proofBytes The SP1 proof bytes
     * @param requiresApproval Whether the key needs owner approval
     * @param expiresAt Expiration timestamp (0 = never expires)
     * @return publicKey The registered key address
     */
    function addValidatorKey(
        bytes calldata publicValues,
        bytes calldata proofBytes,
        bool requiresApproval,
        uint256 expiresAt
    ) external returns (address publicKey);

    /**
     * @notice Registers a sequencer key via signature (for keys without gas)
     * @param publicValues The encoded public values from the attestation
     * @param proofBytes The SP1 proof bytes
     * @param deadline Timestamp after which the signature expires
     * @param signature EIP-712 signature from the TEE key
     * @param requiresApproval Whether the key needs owner approval
     * @param expiresAt Expiration timestamp (0 = never expires)
     * @return publicKey The registered key address
     */
    function addSequencerKeyWithSignature(
        bytes calldata publicValues,
        bytes calldata proofBytes,
        uint256 deadline,
        bytes calldata signature,
        bool requiresApproval,
        uint256 expiresAt
    ) external returns (address publicKey);

    /**
     * @notice Registers a validator key via signature (for keys without gas)
     * @param publicValues The encoded public values from the attestation
     * @param proofBytes The SP1 proof bytes
     * @param deadline Timestamp after which the signature expires
     * @param signature EIP-712 signature from the TEE key
     * @param requiresApproval Whether the key needs owner approval
     * @param expiresAt Expiration timestamp (0 = never expires)
     * @return publicKey The registered key address
     */
    function addValidatorKeyWithSignature(
        bytes calldata publicValues,
        bytes calldata proofBytes,
        uint256 deadline,
        bytes calldata signature,
        bool requiresApproval,
        uint256 expiresAt
    ) external returns (address publicKey);

    /*//////////////////////////////////////////////////////////////
                            KEY APPROVAL
    //////////////////////////////////////////////////////////////*/

    /**
     * @notice Approves a pending sequencer key
     * @param publicKey The pending key to approve
     * @param expiresAt Expiration timestamp (0 = never expires)
     */
    function approveSequencerKey(address publicKey, uint256 expiresAt) external;

    /**
     * @notice Approves a pending validator key
     * @param publicKey The pending key to approve
     * @param expiresAt Expiration timestamp (0 = never expires)
     */
    function approveValidatorKey(address publicKey, uint256 expiresAt) external;

    /**
     * @notice Rejects a pending sequencer key
     * @param publicKey The pending key to reject
     */
    function rejectSequencerKey(address publicKey) external;

    /**
     * @notice Rejects a pending validator key
     * @param publicKey The pending key to reject
     */
    function rejectValidatorKey(address publicKey) external;

    /*//////////////////////////////////////////////////////////////
                            KEY MANAGEMENT
    //////////////////////////////////////////////////////////////*/

    /**
     * @notice Removes a sequencer key
     * @param publicKey The key to remove
     */
    function removeSequencerKey(address publicKey) external;

    /**
     * @notice Removes a validator key
     * @param publicKey The key to remove
     */
    function removeValidatorKey(address publicKey) external;

    /**
     * @notice Sets expiration for a key
     * @param publicKey The key to update
     * @param expiresAt New expiration timestamp (0 = never expires)
     */
    function setKeyExpiration(address publicKey, uint256 expiresAt) external;

    /**
     * @notice Revokes all registered keys
     */
    function revokeAllKeys() external;

    /**
     * @notice Updates the attestation verifier contract
     * @param attestationVerifier The new attestation verifier
     */
    function updateAttestationVerifier(IAttestationVerifier attestationVerifier) external;

    /*//////////////////////////////////////////////////////////////
                            VIEW FUNCTIONS
    //////////////////////////////////////////////////////////////*/

    /**
     * @notice Gets all currently valid sequencer keys
     * @return Array of valid sequencer key addresses
     */
    function getSequencerKeys() external view returns (address[] memory);

    /**
     * @notice Gets all currently valid validator keys
     * @return Array of valid validator key addresses
     */
    function getValidatorKeys() external view returns (address[] memory);

    /**
     * @notice Gets the count of valid sequencer keys
     * @return Number of currently valid sequencer keys
     */
    function sequencerKeyCount() external view returns (uint256);

    /**
     * @notice Gets the count of valid validator keys
     * @return Number of currently valid validator keys
     */
    function validatorKeyCount() external view returns (uint256);

    /**
     * @notice Gets the expiration timestamp for a key
     * @param publicKey The key to check
     * @return Expiration timestamp (0 = never expires)
     */
    function keyExpiration(address publicKey) external view returns (uint256);

    /**
     * @notice Checks if a key is a pending sequencer key
     * @param publicKey The key to check
     * @return True if pending
     */
    function isSequencerKeyPending(address publicKey) external view returns (bool);

    /**
     * @notice Checks if a key is a pending validator key
     * @param publicKey The key to check
     * @return True if pending
     */
    function isValidatorKeyPending(address publicKey) external view returns (bool);
}
