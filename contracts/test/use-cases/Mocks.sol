// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {ERC20} from "@openzeppelin/contracts/token/ERC20/ERC20.sol";
import {ERC721} from "@openzeppelin/contracts/token/ERC721/ERC721.sol";

/// @notice Mock WETH contract that supports deposit and withdraw
contract MockWETH is ERC20 {
    event Deposit(address indexed dst, uint256 wad);
    event Withdrawal(address indexed src, uint256 wad);

    constructor() ERC20("Wrapped Ether", "WETH") {}

    function deposit() public payable {
        _mint(msg.sender, msg.value);
        emit Deposit(msg.sender, msg.value);
    }

    function withdraw(uint256 wad) public {
        require(balanceOf(msg.sender) >= wad, "Insufficient balance");
        _burn(msg.sender, wad);
        payable(msg.sender).transfer(wad);
        emit Withdrawal(msg.sender, wad);
    }

    receive() external payable {
        deposit();
    }
}

/// @notice Mock ERC20 token for testing
contract MockERC20 is ERC20 {
    constructor(string memory name, string memory symbol) ERC20(name, symbol) {
        _mint(msg.sender, 1_000_000 * 10 ** 18);
    }

    function mint(address to, uint256 amount) external {
        _mint(to, amount);
    }
}

/// @notice Mock NFT contract for testing minting scenarios
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
        MockWETH(wethToken).transferFrom(msg.sender, address(this), amount);
        uint256 tokenId = _tokenIdCounter++;
        _mint(to, tokenId);
        emit NFTMinted(to, tokenId, amount);
        return tokenId;
    }
}

/// @notice Mock cross-chain bridge target
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

/// @notice Helper contract to receive ETH
contract ETHReceiver {
    event ETHReceived(address indexed from, uint256 amount);

    receive() external payable {
        emit ETHReceived(msg.sender, msg.value);
    }

    function getBalance() external view returns (uint256) {
        return address(this).balance;
    }
}
