// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {ERC20} from "@openzeppelin/contracts/token/ERC20/ERC20.sol";

/**
 * @title MockSuperchainERC20
 * @notice Mock implementation of Optimism's SuperchainERC20 (ERC-7802) for testing
 * @dev Implements cross-chain mint/burn functionality for Superchain asset interoperability
 *
 * INTERFACE OVERVIEW:
 * - SuperchainERC20 enables tokens to move across the Superchain by burning on source chain
 *   and minting equivalent amount on destination chain
 * - Based on ERC-7802 standard
 * - Bridge calls crosschainMint/crosschainBurn for cross-chain transfers
 *
 * Reference: https://docs.optimism.io/stack/interop/assets/superchain-erc20
 */
contract MockSuperchainERC20 is ERC20 {
    address public immutable SUPERCHAIN_TOKEN_BRIDGE;

    event CrosschainMint(address indexed to, uint256 amount, address indexed sender);
    event CrosschainBurn(address indexed from, uint256 amount, address indexed sender);

    error OnlySuperchainTokenBridge();

    constructor(string memory name, string memory symbol, address bridge) ERC20(name, symbol) {
        SUPERCHAIN_TOKEN_BRIDGE = bridge;
        _mint(msg.sender, 1_000_000 * 10 ** 18); // Initial supply for testing
    }

    /// @notice Mints tokens on destination chain (called by SuperchainTokenBridge)
    /// @param to Recipient address
    /// @param amount Amount to mint
    function crosschainMint(address to, uint256 amount) external {
        if (msg.sender != SUPERCHAIN_TOKEN_BRIDGE) {
            revert OnlySuperchainTokenBridge();
        }

        _mint(to, amount);
        emit CrosschainMint(to, amount, msg.sender);
    }

    /// @notice Burns tokens on source chain (called by SuperchainTokenBridge)
    /// @param from Address to burn from
    /// @param amount Amount to burn
    function crosschainBurn(address from, uint256 amount) external {
        if (msg.sender != SUPERCHAIN_TOKEN_BRIDGE) {
            revert OnlySuperchainTokenBridge();
        }

        _burn(from, amount);
        emit CrosschainBurn(from, amount, msg.sender);
    }

    /// @notice Helper function for testing - allows minting without bridge
    function mint(address to, uint256 amount) external {
        _mint(to, amount);
    }
}
