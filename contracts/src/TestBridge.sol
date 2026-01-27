// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

/**
 * @title TestBridge
 * @notice Minimal test bridge for local development and testing
 * @dev Emits Deposit and Withdrawal events matching synddb-chain-monitor event definitions.
 *      This contract is for testing purposes only and does NOT handle real tokens.
 */
contract TestBridge {
    /// @notice Emitted when a deposit is made (for testing chain monitor)
    /// @param from The address making the deposit
    /// @param to The destination address on L2
    /// @param amount Amount of tokens deposited
    /// @param data Optional additional data
    event Deposit(address indexed from, address indexed to, uint256 amount, bytes data);

    /// @notice Emitted when a withdrawal is processed (for testing chain monitor)
    /// @param from The address requesting the withdrawal
    /// @param amount Amount of tokens withdrawn
    /// @param recipient The recipient address on L1
    /// @param data Optional additional data
    event Withdrawal(address indexed from, uint256 amount, address recipient, bytes data);

    /// @notice Simulate a deposit for testing
    /// @param to Destination address on L2
    /// @param amount Amount being deposited
    function deposit(address to, uint256 amount) external {
        emit Deposit(msg.sender, to, amount, "");
    }

    /// @notice Simulate a deposit with data for testing
    /// @param to Destination address on L2
    /// @param amount Amount being deposited
    /// @param data Additional data payload
    function depositWithData(address to, uint256 amount, bytes calldata data) external {
        emit Deposit(msg.sender, to, amount, data);
    }

    /// @notice Simulate a withdrawal for testing
    /// @param amount Amount being withdrawn
    /// @param recipient Recipient address on L1
    function withdraw(uint256 amount, address recipient) external {
        emit Withdrawal(msg.sender, amount, recipient, "");
    }

    /// @notice Simulate a withdrawal with data for testing
    /// @param amount Amount being withdrawn
    /// @param recipient Recipient address on L1
    /// @param data Additional data payload
    function withdrawWithData(uint256 amount, address recipient, bytes calldata data) external {
        emit Withdrawal(msg.sender, amount, recipient, data);
    }
}
