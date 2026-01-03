// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

/**
 * @title ITeeKeyManager
 * @notice Interface for managing TEE signing keys
 * @dev Validates and tracks signing keys from trusted execution environments
 */
interface ITeeKeyManager {
    /**
     * @notice Checks if a public key is valid (belongs to a verified TEE)
     * @param publicKey The address to check
     * @return True if the key is valid, reverts otherwise
     */
    function isKeyValid(address publicKey) external view returns (bool);

    /**
     * @notice Registers a new TEE key with an attestation proof
     * @dev Anyone can add a key by providing a valid attestation proof
     * @param publicValues The encoded public values from the attestation
     * @param proofBytes The SP1 proof bytes
     */
    function addKey(bytes calldata publicValues, bytes calldata proofBytes) external;

    /**
     * @notice Registers a new TEE key via signature (for keys without gas)
     * @dev Allows a relayer to submit on behalf of a TEE key
     * @param publicValues The encoded public values from the attestation
     * @param proofBytes The SP1 proof bytes
     * @param deadline Timestamp after which the signature expires
     * @param signature EIP-712 signature from the TEE key
     */
    function addKeyWithSignature(
        bytes calldata publicValues,
        bytes calldata proofBytes,
        uint256 deadline,
        bytes calldata signature
    ) external;

    /**
     * @notice Removes a single key from the valid set
     * @dev Only callable by contract owner
     * @param publicKey The key to remove
     */
    function removeKey(address publicKey) external;

    /**
     * @notice Gets all currently valid keys
     * @return Array of valid key addresses
     */
    function getValidKeys() external view returns (address[] memory);

    /**
     * @notice Gets the count of valid keys
     * @return Number of currently valid keys
     */
    function keyCount() external view returns (uint256);
}
