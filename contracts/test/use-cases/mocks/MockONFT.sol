// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {ERC721} from "@openzeppelin/contracts/token/ERC721/ERC721.sol";

/**
 * @title MockONFT
 * @notice Mock implementation of LayerZero's ONFT (Omnichain NFT) for testing
 * @dev Implements cross-chain NFT functionality using burn-and-mint pattern
 *
 * INTERFACE OVERVIEW:
 * - ONFT enables NFTs to move across chains by burning on source chain
 *   and minting equivalent token on destination chain
 * - Based on LayerZero V2 standard
 * - Bridge calls crosschainBurn/crosschainMint for cross-chain transfers
 *
 * Reference: https://docs.layerzero.network/v2/developers/evm/onft/quickstart
 */
contract MockONFT is ERC721 {
    address public immutable CROSS_CHAIN_BRIDGE;
    uint256 private _nextTokenId;

    event CrosschainMint(address indexed to, uint256 indexed tokenId, address indexed sender);
    event CrosschainBurn(address indexed from, uint256 indexed tokenId, address indexed sender);

    error OnlyCrossChainBridge();
    error TokenDoesNotExist();

    constructor(string memory name, string memory symbol, address bridge) ERC721(name, symbol) {
        CROSS_CHAIN_BRIDGE = bridge;
        _nextTokenId = 0;
    }

    /// @notice Mints NFT on destination chain (called by cross-chain bridge)
    /// @param to Recipient address
    /// @param tokenId Token ID to mint
    function crosschainMint(address to, uint256 tokenId) external {
        if (msg.sender != CROSS_CHAIN_BRIDGE) {
            revert OnlyCrossChainBridge();
        }

        _mint(to, tokenId);
        emit CrosschainMint(to, tokenId, msg.sender);
    }

    /// @notice Burns NFT on source chain (called by cross-chain bridge)
    /// @param from Address to burn from
    /// @param tokenId Token ID to burn
    function crosschainBurn(address from, uint256 tokenId) external {
        if (msg.sender != CROSS_CHAIN_BRIDGE) {
            revert OnlyCrossChainBridge();
        }

        if (ownerOf(tokenId) != from) {
            revert TokenDoesNotExist();
        }

        _burn(tokenId);
        emit CrosschainBurn(from, tokenId, msg.sender);
    }

    /// @notice Helper function for testing - allows minting without bridge
    /// @param to Address to mint to
    /// @return tokenId The minted token ID
    function mint(address to) external returns (uint256) {
        uint256 tokenId = _nextTokenId++;
        _mint(to, tokenId);
        return tokenId;
    }

    /// @notice Helper function for testing - allows minting specific token ID without bridge
    /// @param to Address to mint to
    /// @param tokenId Specific token ID to mint
    function mintTokenId(address to, uint256 tokenId) external {
        _mint(to, tokenId);
    }
}
