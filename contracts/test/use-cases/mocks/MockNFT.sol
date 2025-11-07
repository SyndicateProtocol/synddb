// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {ERC721} from "@openzeppelin/contracts/token/ERC721/ERC721.sol";

/**
 * @title MockNFT
 * @notice Mock NFT contract for testing minting scenarios
 */
contract MockNFT is ERC721 {
    uint256 private _tokenIdCounter;
    uint256 public mintPrice;

    event NFTMinted(address indexed to, uint256 indexed tokenId, uint256 price);

    constructor(string memory name, string memory symbol, uint256 _mintPrice) ERC721(name, symbol) {
        mintPrice = _mintPrice;
    }

    /// @notice Free mint function
    function mint(address to) external returns (uint256) {
        uint256 tokenId = _tokenIdCounter++;
        _mint(to, tokenId);
        emit NFTMinted(to, tokenId, 0);
        return tokenId;
    }

    /// @notice Paid mint function (requires ETH)
    function mintWithPayment(address to) external payable returns (uint256) {
        require(msg.value >= mintPrice, "Insufficient payment");
        uint256 tokenId = _tokenIdCounter++;
        _mint(to, tokenId);
        emit NFTMinted(to, tokenId, msg.value);
        return tokenId;
    }

    /// @notice Mint function that accepts WETH payment
    function mintWithWETH(address to, address payable wethToken, uint256 amount) external returns (uint256) {
        require(amount >= mintPrice, "Insufficient payment");
        // Transfer WETH from msg.sender
        (bool success,) = wethToken.call(
            abi.encodeWithSignature("transferFrom(address,address,uint256)", msg.sender, address(this), amount)
        );
        require(success, "WETH transfer failed");
        uint256 tokenId = _tokenIdCounter++;
        _mint(to, tokenId);
        emit NFTMinted(to, tokenId, amount);
        return tokenId;
    }
}
