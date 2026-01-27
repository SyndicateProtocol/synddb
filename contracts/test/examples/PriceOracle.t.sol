// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {Test, console} from "forge-std/Test.sol";
import {PriceOracle} from "src/examples/PriceOracle.sol";

contract PriceOracleTest is Test {
    PriceOracle public oracle;

    address public admin = makeAddr("admin");
    address public bridge = makeAddr("bridge");
    address public user = makeAddr("user");

    // Events to test
    event PriceUpdated(string indexed asset, uint256 price, uint256 timestamp, address indexed source);

    event PriceRequested(bytes32 indexed requestId, string indexed asset, address indexed requester, uint256 maxAge);

    event PriceRequestFulfilled(bytes32 indexed requestId, string indexed asset, uint256 price);

    function setUp() public {
        oracle = new PriceOracle(admin, bridge);
    }

    // ============ Constructor Tests ============

    function test_Constructor_SetsRoles() public view {
        assertTrue(oracle.hasRole(oracle.DEFAULT_ADMIN_ROLE(), admin));
        assertTrue(oracle.hasRole(oracle.UPDATER_ROLE(), admin));
        assertTrue(oracle.hasRole(oracle.UPDATER_ROLE(), bridge));
    }

    function test_Constructor_RevertWhen_AdminIsZero() public {
        vm.expectRevert(PriceOracle.ZeroAddressNotAllowed.selector);
        new PriceOracle(address(0), bridge);
    }

    function test_Constructor_AllowsZeroBridge() public {
        // Should not revert - bridge can be zero (no external updater)
        PriceOracle oracleNoBridge = new PriceOracle(admin, address(0));
        assertTrue(oracleNoBridge.hasRole(oracleNoBridge.DEFAULT_ADMIN_ROLE(), admin));
    }

    // ============ Push Model: updatePrice Tests ============

    function test_UpdatePrice_Success() public {
        vm.prank(bridge);

        vm.expectEmit(true, true, true, true);
        emit PriceUpdated("BTC", 50000_00000000, 1703500000, bridge);

        oracle.updatePrice("BTC", 50000_00000000, 1703500000);

        (uint256 price, uint256 timestamp, uint256 updatedAt) = oracle.getPrice("BTC");
        assertEq(price, 50000_00000000);
        assertEq(timestamp, 1703500000);
        assertEq(updatedAt, block.timestamp);
    }

    function test_UpdatePrice_TracksNewAsset() public {
        assertEq(oracle.getAssetCount(), 0);

        vm.prank(bridge);
        oracle.updatePrice("ETH", 3000_00000000, 1703500000);

        assertEq(oracle.getAssetCount(), 1);
        assertTrue(oracle.hasPrice("ETH"));
    }

    function test_UpdatePrice_RevertWhen_NotUpdater() public {
        vm.prank(user);
        vm.expectRevert();
        oracle.updatePrice("BTC", 50000_00000000, 1703500000);
    }

    function test_UpdatePrice_RevertWhen_ZeroPrice() public {
        vm.prank(bridge);
        vm.expectRevert(PriceOracle.InvalidPrice.selector);
        oracle.updatePrice("BTC", 0, 1703500000);
    }

    function test_UpdatePrice_OverwritesExisting() public {
        vm.startPrank(bridge);

        oracle.updatePrice("BTC", 50000_00000000, 1703500000);
        oracle.updatePrice("BTC", 51000_00000000, 1703500100);

        vm.stopPrank();

        (uint256 price, uint256 timestamp,) = oracle.getPrice("BTC");
        assertEq(price, 51000_00000000);
        assertEq(timestamp, 1703500100);

        // Should still only have 1 asset
        assertEq(oracle.getAssetCount(), 1);
    }

    // ============ Push Model: batchUpdatePrices Tests ============

    function test_BatchUpdatePrices_Success() public {
        string[] memory assets = new string[](3);
        assets[0] = "BTC";
        assets[1] = "ETH";
        assets[2] = "SOL";

        uint256[] memory priceValues = new uint256[](3);
        priceValues[0] = 50000_00000000;
        priceValues[1] = 3000_00000000;
        priceValues[2] = 100_00000000;

        uint256[] memory timestamps = new uint256[](3);
        timestamps[0] = 1703500000;
        timestamps[1] = 1703500001;
        timestamps[2] = 1703500002;

        vm.prank(bridge);
        oracle.batchUpdatePrices(assets, priceValues, timestamps);

        assertEq(oracle.getAssetCount(), 3);

        (uint256 btcPrice,,) = oracle.getPrice("BTC");
        (uint256 ethPrice,,) = oracle.getPrice("ETH");
        (uint256 solPrice,,) = oracle.getPrice("SOL");

        assertEq(btcPrice, 50000_00000000);
        assertEq(ethPrice, 3000_00000000);
        assertEq(solPrice, 100_00000000);
    }

    function test_BatchUpdatePrices_SkipsZeroPrices() public {
        string[] memory assets = new string[](2);
        assets[0] = "BTC";
        assets[1] = "ETH";

        uint256[] memory priceValues = new uint256[](2);
        priceValues[0] = 50000_00000000;
        priceValues[1] = 0; // Should be skipped

        uint256[] memory timestamps = new uint256[](2);
        timestamps[0] = 1703500000;
        timestamps[1] = 1703500001;

        vm.prank(bridge);
        oracle.batchUpdatePrices(assets, priceValues, timestamps);

        assertTrue(oracle.hasPrice("BTC"));
        assertFalse(oracle.hasPrice("ETH"));
    }

    // ============ Pull Model: requestPrice Tests ============

    function test_RequestPrice_EmitsEvent() public {
        vm.prank(user);

        // We can't predict the exact requestId, so just check the event is emitted
        vm.expectEmit(false, true, true, true);
        emit PriceRequested(bytes32(0), "BTC", user, 300);

        bytes32 requestId = oracle.requestPrice("BTC", 300);
        assertTrue(requestId != bytes32(0));
    }

    function test_RequestPrice_IncrementsNonce() public {
        uint256 nonceBefore = oracle.requestNonce();

        vm.prank(user);
        oracle.requestPrice("BTC", 300);

        assertEq(oracle.requestNonce(), nonceBefore + 1);
    }

    function test_RequestPrice_UniqueIds() public {
        vm.startPrank(user);

        bytes32 id1 = oracle.requestPrice("BTC", 300);
        bytes32 id2 = oracle.requestPrice("BTC", 300);
        bytes32 id3 = oracle.requestPrice("ETH", 300);

        vm.stopPrank();

        assertTrue(id1 != id2);
        assertTrue(id2 != id3);
        assertTrue(id1 != id3);
    }

    // ============ Pull Model: batchRequestPrices Tests ============

    function test_BatchRequestPrices_Success() public {
        string[] memory assets = new string[](3);
        assets[0] = "BTC";
        assets[1] = "ETH";
        assets[2] = "SOL";

        vm.prank(user);
        bytes32[] memory requestIds = oracle.batchRequestPrices(assets, 300);

        assertEq(requestIds.length, 3);
        assertTrue(requestIds[0] != requestIds[1]);
        assertTrue(requestIds[1] != requestIds[2]);
    }

    // ============ fulfillPriceRequest Tests ============

    function test_FulfillPriceRequest_EmitsBothEvents() public {
        // First request a price
        vm.prank(user);
        bytes32 requestId = oracle.requestPrice("BTC", 300);

        // Then fulfill it
        vm.prank(bridge);

        vm.expectEmit(true, true, true, true);
        emit PriceUpdated("BTC", 50000_00000000, 1703500000, bridge);

        vm.expectEmit(true, true, true, true);
        emit PriceRequestFulfilled(requestId, "BTC", 50000_00000000);

        oracle.fulfillPriceRequest(requestId, "BTC", 50000_00000000, 1703500000);

        // Price should be stored
        (uint256 price,,) = oracle.getPrice("BTC");
        assertEq(price, 50000_00000000);
    }

    // ============ Read Function Tests ============

    function test_GetPrice_RevertWhen_AssetNotFound() public {
        vm.expectRevert(abi.encodeWithSelector(PriceOracle.AssetNotFound.selector, "UNKNOWN"));
        oracle.getPrice("UNKNOWN");
    }

    function test_GetPriceIfFresh_Success() public {
        vm.prank(bridge);
        oracle.updatePrice("BTC", 50000_00000000, 1703500000);

        uint256 price = oracle.getPriceIfFresh("BTC", 3600); // 1 hour max age
        assertEq(price, 50000_00000000);
    }

    function test_GetPriceIfFresh_RevertWhen_Stale() public {
        vm.prank(bridge);
        oracle.updatePrice("BTC", 50000_00000000, 1703500000);

        // Advance time by 2 hours
        vm.warp(block.timestamp + 7200);

        vm.expectRevert(abi.encodeWithSelector(PriceOracle.StalePrice.selector, 7200, 3600));
        oracle.getPriceIfFresh("BTC", 3600); // 1 hour max age
    }

    function test_GetPriceIfFresh_ZeroMaxAge_AcceptsAny() public {
        vm.prank(bridge);
        oracle.updatePrice("BTC", 50000_00000000, 1703500000);

        // Advance time significantly
        vm.warp(block.timestamp + 365 days);

        // maxAge = 0 means any age is acceptable
        uint256 price = oracle.getPriceIfFresh("BTC", 0);
        assertEq(price, 50000_00000000);
    }

    function test_GetAllAssets_ReturnsSymbols() public {
        vm.startPrank(bridge);
        oracle.updatePrice("BTC", 50000_00000000, 1703500000);
        oracle.updatePrice("ETH", 3000_00000000, 1703500000);
        oracle.updatePrice("SOL", 100_00000000, 1703500000);
        vm.stopPrank();

        string[] memory assets = oracle.getAllAssets();
        assertEq(assets.length, 3);
        assertEq(assets[0], "BTC");
        assertEq(assets[1], "ETH");
        assertEq(assets[2], "SOL");
    }

    // ============ Fuzz Tests ============

    function testFuzz_UpdatePrice(uint256 price, uint256 timestamp) public {
        // Bound to valid values
        price = bound(price, 1, type(uint128).max);
        timestamp = bound(timestamp, 1, block.timestamp);

        vm.prank(bridge);
        oracle.updatePrice("BTC", price, timestamp);

        (uint256 storedPrice, uint256 storedTimestamp,) = oracle.getPrice("BTC");
        assertEq(storedPrice, price);
        assertEq(storedTimestamp, timestamp);
    }

    function testFuzz_RequestPrice_AlwaysUniqueIds(uint8 count) public {
        count = uint8(bound(count, 2, 50));

        bytes32[] memory ids = new bytes32[](count);

        vm.startPrank(user);
        for (uint256 i = 0; i < count; i++) {
            ids[i] = oracle.requestPrice("BTC", 300);
        }
        vm.stopPrank();

        // Check all IDs are unique
        for (uint256 i = 0; i < count; i++) {
            for (uint256 j = i + 1; j < count; j++) {
                assertTrue(ids[i] != ids[j], "Duplicate request ID found");
            }
        }
    }
}
