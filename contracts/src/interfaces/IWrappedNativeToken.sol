// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

/**
 * @title IWrappedNativeToken
 * @notice Interface for Wrapped Native Token contract (e.g., WETH on Ethereum, WMATIC on Polygon)
 */
interface IWrappedNativeToken {
    /// @notice Deposit native token and receive wrapped native token
    function deposit() external payable;

    /// @notice Withdraw wrapped native token and receive native token
    /// @param amount The amount of wrapped native token to withdraw
    function withdraw(uint256 amount) external;

    /// @notice Transfer wrapped native tokens
    /// @param to The recipient address
    /// @param amount The amount to transfer
    /// @return success Whether the transfer succeeded
    function transfer(address to, uint256 amount) external returns (bool success);

    /// @notice Get wrapped native token balance of an account
    /// @param account The account to query
    /// @return balance The wrapped native token balance
    function balanceOf(address account) external view returns (uint256 balance);
}
