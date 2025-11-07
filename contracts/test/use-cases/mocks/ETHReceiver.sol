// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

/**
 * @title ETHReceiver
 * @notice Helper contract to receive ETH for testing
 */
contract ETHReceiver {
    event ETHReceived(address indexed from, uint256 amount);

    receive() external payable {
        emit ETHReceived(msg.sender, msg.value);
    }

    function getBalance() external view returns (uint256) {
        return address(this).balance;
    }
}
