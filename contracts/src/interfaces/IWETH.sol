// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

/**
 * @title IWETH
 * @notice Interface for Wrapped Ether (WETH) contract
 */
interface IWETH {
    /// @notice Deposit ETH and receive WETH
    function deposit() external payable;

    /// @notice Withdraw WETH and receive ETH
    /// @param amount The amount of WETH to withdraw
    function withdraw(uint256 amount) external;

    /// @notice Transfer WETH tokens
    /// @param to The recipient address
    /// @param amount The amount to transfer
    /// @return success Whether the transfer succeeded
    function transfer(address to, uint256 amount) external returns (bool success);

    /// @notice Get WETH balance of an account
    /// @param account The account to query
    /// @return balance The WETH balance
    function balanceOf(address account) external view returns (uint256 balance);
}
