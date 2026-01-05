// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {AccessControl} from "@openzeppelin/contracts/access/AccessControl.sol";

/**
 * @title PriceOracle
 * @notice On-chain price oracle that demonstrates bidirectional message passing with SyndDB
 * @dev Supports two message flows:
 *      - Push model (outbound): Off-chain app pushes prices via Bridge.handleMessage() -> updatePrice()
 *      - Pull model (inbound): Contract emits PriceRequested, app listens and responds with price update
 */
contract PriceOracle is AccessControl {
    /// @notice Role that can update prices (typically the Bridge contract)
    bytes32 public constant UPDATER_ROLE = keccak256("UPDATER_ROLE");

    /// @notice Role that can request prices (anyone by default, but can be restricted)
    bytes32 public constant REQUESTER_ROLE = keccak256("REQUESTER_ROLE");

    /// @notice Stored price data for an asset
    struct Price {
        uint256 price; // Price in base units (e.g., USD with 8 decimals)
        uint256 timestamp; // Unix timestamp of the price
        uint256 updatedAt; // Block timestamp when stored
    }

    /// @notice Mapping from asset symbol hash to price data
    /// @dev Key is keccak256(bytes(asset)), e.g., keccak256("BTC")
    mapping(bytes32 => Price) public prices;

    /// @notice Nonce for request IDs to ensure uniqueness
    uint256 public requestNonce;

    // ============ Events ============

    /**
     * @notice Emitted when a price is updated (push model completion)
     * @param asset Asset symbol (e.g., "BTC", "ETH")
     * @param price Price in base units
     * @param timestamp Unix timestamp of the price data
     * @param source Address that submitted the update
     */
    event PriceUpdated(string indexed asset, uint256 price, uint256 timestamp, address indexed source);

    /**
     * @notice Emitted when a price is requested (pull model initiation)
     * @dev The off-chain application should listen for this event and respond
     *      by submitting a price update through the Bridge
     * @param requestId Unique identifier for this request
     * @param asset Asset symbol being requested
     * @param requester Address that made the request
     * @param maxAge Maximum acceptable age of the price data (0 = any age)
     */
    event PriceRequested(bytes32 indexed requestId, string indexed asset, address indexed requester, uint256 maxAge);

    /**
     * @notice Emitted when a price request is fulfilled
     * @param requestId The request ID that was fulfilled
     * @param asset Asset symbol
     * @param price The fulfilled price
     */
    event PriceRequestFulfilled(bytes32 indexed requestId, string indexed asset, uint256 price);

    // ============ Errors ============

    error ZeroAddressNotAllowed();
    error StalePrice(uint256 priceAge, uint256 maxAge);
    error InvalidPrice();
    error FutureTimestamp(uint256 timestamp, uint256 currentTime);
    error ArrayLengthMismatch();
    error AssetNotFound(string asset);

    // ============ Constructor ============

    /**
     * @notice Initializes the price oracle
     * @param admin Address to receive admin role
     * @param bridge Address of the Bridge contract (receives UPDATER_ROLE)
     */
    constructor(address admin, address bridge) {
        if (admin == address(0)) revert ZeroAddressNotAllowed();

        _grantRole(DEFAULT_ADMIN_ROLE, admin);
        _grantRole(UPDATER_ROLE, admin);

        // Grant updater role to bridge if provided
        if (bridge != address(0)) {
            _grantRole(UPDATER_ROLE, bridge);
        }

        // By default, anyone can request prices (no role check)
        // To restrict, grant REQUESTER_ROLE to specific addresses
        // and modify requestPrice to check the role
    }

    // ============ Push Model: Receive Price Updates ============

    /**
     * @notice Update the price for an asset (push model)
     * @dev Called by the Bridge contract when processing a message from the off-chain app.
     *      The Bridge calls this via: handleMessage(messageId, signature) which executes
     *      the payload: abi.encodeCall(PriceOracle.updatePrice, (asset, price, timestamp))
     * @param asset Asset symbol (e.g., "BTC", "ETH")
     * @param price Price in base units (e.g., 50000_00000000 for $50,000.00 with 8 decimals)
     * @param timestamp Unix timestamp when the price was observed off-chain
     */
    function updatePrice(string calldata asset, uint256 price, uint256 timestamp) external onlyRole(UPDATER_ROLE) {
        if (price == 0) revert InvalidPrice();
        if (timestamp > block.timestamp) revert FutureTimestamp(timestamp, block.timestamp);

        bytes32 assetHash = keccak256(bytes(asset));
        prices[assetHash] = Price({price: price, timestamp: timestamp, updatedAt: block.timestamp});

        emit PriceUpdated(asset, price, timestamp, msg.sender);
    }

    /**
     * @notice Update the price and mark a request as fulfilled
     * @dev Same as updatePrice but also emits PriceRequestFulfilled event
     * @param requestId The request ID being fulfilled
     * @param asset Asset symbol
     * @param price Price in base units
     * @param timestamp Unix timestamp of the price
     */
    function fulfillPriceRequest(bytes32 requestId, string calldata asset, uint256 price, uint256 timestamp)
        external
        onlyRole(UPDATER_ROLE)
    {
        if (price == 0) revert InvalidPrice();
        if (timestamp > block.timestamp) revert FutureTimestamp(timestamp, block.timestamp);

        bytes32 assetHash = keccak256(bytes(asset));
        prices[assetHash] = Price({price: price, timestamp: timestamp, updatedAt: block.timestamp});

        emit PriceUpdated(asset, price, timestamp, msg.sender);
        emit PriceRequestFulfilled(requestId, asset, price);
    }

    /**
     * @notice Batch update multiple prices
     * @param assets Array of asset symbols
     * @param priceValues Array of prices
     * @param timestamps Array of timestamps
     */
    function batchUpdatePrices(string[] calldata assets, uint256[] calldata priceValues, uint256[] calldata timestamps)
        external
        onlyRole(UPDATER_ROLE)
    {
        if (assets.length != priceValues.length || assets.length != timestamps.length) {
            revert ArrayLengthMismatch();
        }

        for (uint256 i = 0; i < assets.length; i++) {
            if (priceValues[i] == 0) continue; // Skip invalid prices
            if (timestamps[i] > block.timestamp) continue; // Skip future timestamps

            bytes32 assetHash = keccak256(bytes(assets[i]));
            prices[assetHash] = Price({price: priceValues[i], timestamp: timestamps[i], updatedAt: block.timestamp});

            emit PriceUpdated(assets[i], priceValues[i], timestamps[i], msg.sender);
        }
    }

    // ============ Pull Model: Request Prices ============

    /**
     * @notice Request a price update for an asset (pull model)
     * @dev Emits PriceRequested event that the off-chain application should monitor.
     *      The app then fetches the price and submits it via the Bridge.
     * @param asset Asset symbol to request
     * @param maxAge Maximum acceptable age in seconds (0 = any age acceptable)
     * @return requestId Unique identifier for this request
     */
    function requestPrice(string calldata asset, uint256 maxAge) external returns (bytes32 requestId) {
        requestId = keccak256(
            abi.encodePacked(block.chainid, address(this), requestNonce++, asset, msg.sender, block.timestamp)
        );

        emit PriceRequested(requestId, asset, msg.sender, maxAge);
    }

    /**
     * @notice Request prices for multiple assets
     * @param assets Array of asset symbols
     * @param maxAge Maximum acceptable age for all requests
     * @return requestIds Array of request IDs
     */
    function batchRequestPrices(string[] calldata assets, uint256 maxAge)
        external
        returns (bytes32[] memory requestIds)
    {
        requestIds = new bytes32[](assets.length);

        for (uint256 i = 0; i < assets.length; i++) {
            requestIds[i] = keccak256(
                abi.encodePacked(block.chainid, address(this), requestNonce++, assets[i], msg.sender, block.timestamp)
            );

            emit PriceRequested(requestIds[i], assets[i], msg.sender, maxAge);
        }
    }

    // ============ Read Functions ============

    /**
     * @notice Get the latest price for an asset
     * @param asset Asset symbol
     * @return price The price value
     * @return timestamp When the price was observed
     * @return updatedAt When the price was stored on-chain
     */
    function getPrice(string calldata asset)
        external
        view
        returns (uint256 price, uint256 timestamp, uint256 updatedAt)
    {
        bytes32 assetHash = keccak256(bytes(asset));
        Price storage p = prices[assetHash];

        if (p.updatedAt == 0) revert AssetNotFound(asset);

        return (p.price, p.timestamp, p.updatedAt);
    }

    /**
     * @notice Get the latest price with a maximum age check
     * @param asset Asset symbol
     * @param maxAge Maximum acceptable age in seconds
     * @return price The price value
     */
    function getPriceIfFresh(string calldata asset, uint256 maxAge) external view returns (uint256 price) {
        bytes32 assetHash = keccak256(bytes(asset));
        Price storage p = prices[assetHash];

        if (p.updatedAt == 0) revert AssetNotFound(asset);

        uint256 age = block.timestamp - p.updatedAt;
        if (maxAge > 0 && age > maxAge) {
            revert StalePrice(age, maxAge);
        }

        return p.price;
    }

    /**
     * @notice Check if an asset has a stored price
     */
    function hasPrice(string calldata asset) external view returns (bool) {
        bytes32 assetHash = keccak256(bytes(asset));
        return prices[assetHash].updatedAt > 0;
    }
}
