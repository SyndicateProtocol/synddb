// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {MockERC20} from "./MockERC20.sol";

interface ISuperchainERC20 {
    function crosschainMint(address to, uint256 amount) external;
    function crosschainBurn(address from, uint256 amount) external;
}

interface ICrosschainERC20 {
    function mint(address to, uint256 amount, bytes calldata data) external;
    function burn(address from, uint256 amount) external;
}

interface IONFT {
    function crosschainMint(address to, uint256 tokenId) external;
    function crosschainBurn(address from, uint256 tokenId) external;
}

/**
 * @title MockCrossChainReceiver
 * @notice Mock cross-chain bridge target for testing cross-chain message passing
 * @dev Supports multiple common cross-chain token standards:
 *      - SuperchainERC20 (Optimism ERC-7802)
 *      - CrosschainERC20 (generic cross-chain standard)
 *      - ONFT (LayerZero Omnichain NFT)
 *      - Standard ERC20 with mint function
 *
 * This mock simulates how a destination chain bridge would receive and process
 * cross-chain token transfers from various standards.
 */
contract MockCrossChainReceiver {
    event CrossChainMessageReceived(
        bytes32 indexed messageId, address indexed token, address indexed recipient, uint256 amount, bytes data
    );
    event SuperchainMintReceived(bytes32 indexed messageId, address indexed token, address indexed to, uint256 amount);
    event CrosschainMintReceived(bytes32 indexed messageId, address indexed token, address indexed to, uint256 amount);
    event ONFTMintReceived(bytes32 indexed messageId, address indexed token, address indexed to, uint256 tokenId);

    /// @notice Simulates receiving a cross-chain mint message (standard ERC20)
    function receiveMintMessage(
        bytes32 messageId,
        address token,
        address recipient,
        uint256 amount,
        bytes calldata data
    ) external {
        emit CrossChainMessageReceived(messageId, token, recipient, amount, data);

        // Simulate standard ERC20 mint
        MockERC20(token).mint(recipient, amount);
    }

    /// @notice Simulates receiving a SuperchainERC20 cross-chain mint
    /// @dev Uses Optimism's SuperchainERC20 (ERC-7802) interface
    function receiveSuperchainMint(bytes32 messageId, address token, address to, uint256 amount) external {
        emit SuperchainMintReceived(messageId, token, to, amount);

        // Call SuperchainERC20's crosschainMint
        ISuperchainERC20(token).crosschainMint(to, amount);
    }

    /// @notice Simulates receiving a generic CrosschainERC20 mint
    function receiveCrosschainMint(bytes32 messageId, address token, address to, uint256 amount, bytes calldata data)
        external
    {
        emit CrosschainMintReceived(messageId, token, to, amount);

        // Call CrosschainERC20's mint with data
        ICrosschainERC20(token).mint(to, amount, data);
    }

    /// @notice Simulates receiving an ONFT cross-chain mint
    /// @dev Uses LayerZero's ONFT interface
    function receiveONFTMint(bytes32 messageId, address token, address to, uint256 tokenId) external {
        emit ONFTMintReceived(messageId, token, to, tokenId);

        // Call ONFT's crosschainMint
        IONFT(token).crosschainMint(to, tokenId);
    }
}
