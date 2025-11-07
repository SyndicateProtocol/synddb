// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {MockERC20} from "./MockERC20.sol";

/**
 * @title MockCrossChainReceiver
 * @notice Mock cross-chain bridge target for testing cross-chain message passing
 */
contract MockCrossChainReceiver {
    event CrossChainMessageReceived(
        bytes32 indexed messageId, address indexed token, address indexed recipient, uint256 amount, bytes data
    );

    /// @notice Simulates receiving a cross-chain mint message
    function receiveMintMessage(
        bytes32 messageId,
        address token,
        address recipient,
        uint256 amount,
        bytes calldata data
    ) external {
        // In a real scenario, this would mint tokens on the destination chain
        emit CrossChainMessageReceived(messageId, token, recipient, amount, data);

        // Simulate the mint
        MockERC20(token).mint(recipient, amount);
    }
}
