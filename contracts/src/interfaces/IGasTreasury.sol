// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

/**
 * @title IGasTreasury
 * @notice Interface for managing gas funding for TEE keys
 * @dev Provides signature-based funding for keys without gas
 */
interface IGasTreasury {
    /**
     * @notice Funds a registered TEE key via signature
     * @dev Called by relayer with signature from the key requesting funding
     * @param teeKey The key to fund
     * @param deadline Timestamp after which the signature expires
     * @param signature EIP-712 signature from the TEE key
     */
    function fundKeyWithSignature(address teeKey, uint256 deadline, bytes calldata signature) external;

    /**
     * @notice Gets the nonce for a key (for signature replay protection)
     * @param teeKey The key to check
     * @return Current nonce value
     */
    function nonces(address teeKey) external view returns (uint256);

    /**
     * @notice Gets the total amount funded to a key
     * @param teeKey The key to check
     * @return Total amount funded in wei
     */
    function totalFunded(address teeKey) external view returns (uint256);

    /**
     * @notice Gets the maximum funding amount per key
     * @return Max funding in wei
     */
    function maxFundingPerKey() external view returns (uint256);

    /**
     * @notice Gets the funding amount per request
     * @return Funding amount in wei
     */
    function fundingAmount() external view returns (uint256);
}
