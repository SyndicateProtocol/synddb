// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

/**
 * @title PriceOracle
 * @notice On-chain price oracle that receives price updates via Bridge messages
 * @dev Prices are stored as uint256 scaled by 10^18 (1 USD = 1e18)
 */
contract PriceOracle {
    struct Price {
        uint256 price; // Price scaled by 1e18
        uint256 timestamp; // Unix timestamp of the price update
        uint256 blockNumber; // Block when price was updated
    }

    // Mapping from asset symbol hash to price data
    mapping(bytes32 => Price) public prices;

    // Array of known asset hashes for enumeration
    bytes32[] public assetHashes;
    mapping(bytes32 => bool) public knownAsset;

    // Access control
    address public immutable bridge;
    address public owner;

    // Events
    event PriceUpdated(
        string indexed asset, bytes32 indexed assetHash, uint256 price, uint256 timestamp, uint256 blockNumber
    );
    event OwnershipTransferred(address indexed previousOwner, address indexed newOwner);

    // Errors
    error Unauthorized();
    error StalePrice(uint256 newTimestamp, uint256 existingTimestamp);
    error InvalidPrice();
    error InvalidTimestamp();

    modifier onlyBridge() {
        if (msg.sender != bridge) revert Unauthorized();
        _;
    }

    modifier onlyOwner() {
        if (msg.sender != owner) revert Unauthorized();
        _;
    }

    constructor(address _bridge) {
        bridge = _bridge;
        owner = msg.sender;
    }

    /**
     * @notice Update the price of an asset
     * @dev Only callable by the Bridge contract
     * @param asset The asset symbol (e.g., "bitcoin", "ethereum")
     * @param priceScaled The price scaled by 10^18
     * @param timestamp The timestamp of the price observation
     */
    function updatePrice(string calldata asset, uint256 priceScaled, uint256 timestamp) external onlyBridge {
        if (priceScaled == 0) revert InvalidPrice();
        if (timestamp == 0 || timestamp > block.timestamp + 300) revert InvalidTimestamp();

        bytes32 assetHash = keccak256(bytes(asset));
        Price storage current = prices[assetHash];

        // Reject stale prices (allow same timestamp for idempotency)
        if (timestamp < current.timestamp) {
            revert StalePrice(timestamp, current.timestamp);
        }

        // Track new assets
        if (!knownAsset[assetHash]) {
            knownAsset[assetHash] = true;
            assetHashes.push(assetHash);
        }

        // Update price
        current.price = priceScaled;
        current.timestamp = timestamp;
        current.blockNumber = block.number;

        emit PriceUpdated(asset, assetHash, priceScaled, timestamp, block.number);
    }

    /**
     * @notice Get the current price of an asset
     * @param asset The asset symbol
     * @return price The price scaled by 10^18
     * @return timestamp The timestamp of the price
     * @return blockNumber The block when price was updated
     */
    function getPrice(string calldata asset)
        external
        view
        returns (uint256 price, uint256 timestamp, uint256 blockNumber)
    {
        bytes32 assetHash = keccak256(bytes(asset));
        Price storage p = prices[assetHash];
        return (p.price, p.timestamp, p.blockNumber);
    }

    /**
     * @notice Get the current price by asset hash
     * @param assetHash The keccak256 hash of the asset symbol
     * @return price The price scaled by 10^18
     * @return timestamp The timestamp of the price
     * @return blockNumber The block when price was updated
     */
    function getPriceByHash(bytes32 assetHash)
        external
        view
        returns (uint256 price, uint256 timestamp, uint256 blockNumber)
    {
        Price storage p = prices[assetHash];
        return (p.price, p.timestamp, p.blockNumber);
    }

    /**
     * @notice Get the number of tracked assets
     */
    function assetCount() external view returns (uint256) {
        return assetHashes.length;
    }

    /**
     * @notice Transfer ownership
     */
    function transferOwnership(address newOwner) external onlyOwner {
        address oldOwner = owner;
        owner = newOwner;
        emit OwnershipTransferred(oldOwner, newOwner);
    }
}
