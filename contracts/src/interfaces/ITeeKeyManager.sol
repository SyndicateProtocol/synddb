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
}
