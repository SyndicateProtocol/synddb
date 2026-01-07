// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {IAttestationVerifier} from "src/interfaces/IAttestationVerifier.sol";
import {KeyType} from "src/types/DataTypes.sol";

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
     * @notice Checks if a key is valid for the given type (registered and not expired)
     * @param keyType The type of key (Sequencer or Validator)
     * @param publicKey The address to check
     * @return True if the key is valid, reverts otherwise
     */
    function isKeyValid(KeyType keyType, address publicKey) external view returns (bool);

    /*//////////////////////////////////////////////////////////////
                            KEY REGISTRATION
    //////////////////////////////////////////////////////////////*/

    /**
     * @notice Registers a new key with an attestation proof
     * @param keyType The type of key (Sequencer or Validator)
     * @param publicValues The encoded public values from the attestation
     * @param proofBytes The ZK proof bytes
     * @param requiresApproval Whether the key needs owner approval
     * @param expiresAt Expiration timestamp (0 = never expires)
     * @return publicKey The registered key address
     */
    function addKey(
        KeyType keyType,
        bytes calldata publicValues,
        bytes calldata proofBytes,
        bool requiresApproval,
        uint256 expiresAt
    ) external returns (address publicKey);

    /**
     * @notice Registers a key via signature (for keys without gas)
     * @param keyType The type of key (Sequencer or Validator)
     * @param publicValues The encoded public values from the attestation
     * @param proofBytes The ZK proof bytes
     * @param deadline Timestamp after which the signature expires
     * @param signature EIP-712 signature from the TEE key
     * @param requiresApproval Whether the key needs owner approval
     * @param expiresAt Expiration timestamp (0 = never expires)
     * @return publicKey The registered key address
     */
    function addKeyWithSignature(
        KeyType keyType,
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
     * @notice Approves a pending key
     * @param keyType The type of key (Sequencer or Validator)
     * @param publicKey The pending key to approve
     * @param expiresAt Expiration timestamp (0 = never expires)
     */
    function approveKey(KeyType keyType, address publicKey, uint256 expiresAt) external;

    /**
     * @notice Rejects a pending key
     * @param keyType The type of key (Sequencer or Validator)
     * @param publicKey The pending key to reject
     */
    function rejectKey(KeyType keyType, address publicKey) external;

    /*//////////////////////////////////////////////////////////////
                            KEY MANAGEMENT
    //////////////////////////////////////////////////////////////*/

    /**
     * @notice Removes a key
     * @param keyType The type of key (Sequencer or Validator)
     * @param publicKey The key to remove
     */
    function removeKey(KeyType keyType, address publicKey) external;

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
     * @notice Gets all currently valid keys of a given type
     * @param keyType The type of keys to retrieve
     * @return Array of valid key addresses
     */
    function getKeys(KeyType keyType) external view returns (address[] memory);

    /**
     * @notice Gets the count of valid keys of a given type
     * @param keyType The type of keys to count
     * @return Number of currently valid keys
     */
    function keyCount(KeyType keyType) external view returns (uint256);

    /**
     * @notice Gets the expiration timestamp for a key
     * @param publicKey The key to check
     * @return Expiration timestamp (0 = never expires)
     */
    function keyExpiration(address publicKey) external view returns (uint256);

    /**
     * @notice Checks if a key is pending approval
     * @param keyType The type of key (Sequencer or Validator)
     * @param publicKey The key to check
     * @return True if pending
     */
    function isKeyPending(KeyType keyType, address publicKey) external view returns (bool);
}
