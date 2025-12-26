// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {Test} from "forge-std/Test.sol";
import {Bridge} from "src/Bridge.sol";
import {PriceOracle} from "../src/PriceOracle.sol";
import {SequencerSignature} from "src/types/DataTypes.sol";
import {ValidatorSignatureThresholdModule} from "src/modules/ValidatorSignatureThresholdModule.sol";
import {WETH9} from "./mocks/WETH9.sol";
import {MessageHashUtils} from "@openzeppelin/contracts/utils/cryptography/MessageHashUtils.sol";

/**
 * @title PriceOracleTest
 * @notice End-to-end tests for the Price Oracle receiving price updates via Bridge
 */
contract PriceOracleTest is Test {
    Bridge public bridge;
    PriceOracle public oracle;
    ValidatorSignatureThresholdModule public validatorModule;
    WETH9 public weth;

    address public admin;
    address public sequencer;

    uint256[] public validatorPrivateKeys;
    address[] public validators;

    // Price constants (scaled by 1e18)
    uint256 constant BTC_PRICE = 67196_645000000000000000; // ~$67,196.645
    uint256 constant ETH_PRICE = 3514_490000000000000000;  // ~$3,514.49

    function setUp() public {
        admin = address(this);
        sequencer = makeAddr("sequencer");

        // Deploy WETH and Bridge
        weth = new WETH9();
        bridge = new Bridge(admin, address(weth));

        // Setup validators (3 validators, threshold of 2)
        setupValidators(3);
        validatorModule = new ValidatorSignatureThresholdModule(address(bridge), validators, 2);

        // Deploy PriceOracle with Bridge as the authorized caller
        oracle = new PriceOracle(address(bridge));

        // Configure Bridge
        bridge.grantRole(bridge.MESSAGE_INITIALIZER_ROLE(), sequencer);
        bridge.addPreModule(address(validatorModule));
    }

    function setupValidators(uint256 count) internal {
        for (uint256 i = 1; i <= count; i++) {
            uint256 privateKey = i;
            address validatorAddr = vm.addr(privateKey);

            validatorPrivateKeys.push(privateKey);
            validators.push(validatorAddr);

            bridge.grantRole(bridge.VALIDATOR_ROLE(), validatorAddr);
        }
    }

    function signMessage(bytes32 messageId, uint256 privateKey) internal pure returns (bytes memory) {
        bytes32 ethSignedMessageHash = MessageHashUtils.toEthSignedMessageHash(messageId);
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(privateKey, ethSignedMessageHash);
        return abi.encodePacked(r, s, v);
    }

    function submitValidatorSignatures(bytes32 messageId, uint256 count) internal {
        for (uint256 i = 0; i < count; i++) {
            bytes memory sig = signMessage(messageId, validatorPrivateKeys[i]);
            bridge.signMessageWithSignature(messageId, sig);
        }
    }

    /*//////////////////////////////////////////////////////////////
                        BASIC PRICE UPDATE TESTS
    //////////////////////////////////////////////////////////////*/

    function test_UpdateBitcoinPrice() public {
        bytes32 messageId = keccak256("price-update-btc-1");
        uint256 timestamp = block.timestamp;

        // Encode the updatePrice call
        bytes memory payload = abi.encodeWithSelector(
            oracle.updatePrice.selector,
            "bitcoin",
            BTC_PRICE,
            timestamp
        );

        SequencerSignature memory sig = SequencerSignature({
            signature: new bytes(65),
            submittedAt: block.timestamp
        });

        // Initialize message
        vm.prank(sequencer);
        bridge.initializeMessage(messageId, address(oracle), payload, sig, 0);

        // Submit validator signatures (2 of 3)
        submitValidatorSignatures(messageId, 2);

        // Execute message
        bridge.handleMessage(messageId);

        // Verify price was updated
        (uint256 price, uint256 ts, uint256 blockNum) = oracle.getPrice("bitcoin");
        assertEq(price, BTC_PRICE, "BTC price mismatch");
        assertEq(ts, timestamp, "Timestamp mismatch");
        assertEq(blockNum, block.number, "Block number mismatch");
    }

    function test_UpdateMultiplePrices() public {
        uint256 timestamp = block.timestamp;
        SequencerSignature memory sig = SequencerSignature({
            signature: new bytes(65),
            submittedAt: block.timestamp
        });

        // Update Bitcoin price
        bytes32 btcMessageId = keccak256("price-update-btc-2");
        bytes memory btcPayload = abi.encodeWithSelector(
            oracle.updatePrice.selector,
            "bitcoin",
            BTC_PRICE,
            timestamp
        );

        vm.prank(sequencer);
        bridge.initializeMessage(btcMessageId, address(oracle), btcPayload, sig, 0);
        submitValidatorSignatures(btcMessageId, 2);
        bridge.handleMessage(btcMessageId);

        // Update Ethereum price
        bytes32 ethMessageId = keccak256("price-update-eth-1");
        bytes memory ethPayload = abi.encodeWithSelector(
            oracle.updatePrice.selector,
            "ethereum",
            ETH_PRICE,
            timestamp
        );

        vm.prank(sequencer);
        bridge.initializeMessage(ethMessageId, address(oracle), ethPayload, sig, 0);
        submitValidatorSignatures(ethMessageId, 2);
        bridge.handleMessage(ethMessageId);

        // Verify both prices
        (uint256 btcPrice,,) = oracle.getPrice("bitcoin");
        (uint256 ethPrice,,) = oracle.getPrice("ethereum");

        assertEq(btcPrice, BTC_PRICE, "BTC price mismatch");
        assertEq(ethPrice, ETH_PRICE, "ETH price mismatch");
        assertEq(oracle.assetCount(), 2, "Should track 2 assets");
    }

    /*//////////////////////////////////////////////////////////////
                        STALE PRICE REJECTION TESTS
    //////////////////////////////////////////////////////////////*/

    function test_RejectStalePrice() public {
        uint256 timestamp = block.timestamp;
        SequencerSignature memory sig = SequencerSignature({
            signature: new bytes(65),
            submittedAt: block.timestamp
        });

        // First update
        bytes32 messageId1 = keccak256("price-update-stale-1");
        bytes memory payload1 = abi.encodeWithSelector(
            oracle.updatePrice.selector,
            "bitcoin",
            BTC_PRICE,
            timestamp
        );

        vm.prank(sequencer);
        bridge.initializeMessage(messageId1, address(oracle), payload1, sig, 0);
        submitValidatorSignatures(messageId1, 2);
        bridge.handleMessage(messageId1);

        // Try to update with older timestamp (should fail)
        bytes32 messageId2 = keccak256("price-update-stale-2");
        bytes memory payload2 = abi.encodeWithSelector(
            oracle.updatePrice.selector,
            "bitcoin",
            BTC_PRICE + 1000e18,
            timestamp - 1 // Older timestamp
        );

        vm.prank(sequencer);
        bridge.initializeMessage(messageId2, address(oracle), payload2, sig, 0);
        submitValidatorSignatures(messageId2, 2);

        // Message execution should revert due to stale price
        vm.expectRevert();
        bridge.handleMessage(messageId2);

        // Original price should remain
        (uint256 price,,) = oracle.getPrice("bitcoin");
        assertEq(price, BTC_PRICE, "Price should not have changed");
    }

    /*//////////////////////////////////////////////////////////////
                        ACCESS CONTROL TESTS
    //////////////////////////////////////////////////////////////*/

    function test_DirectCallRejected() public {
        // Try to call updatePrice directly (not through Bridge)
        vm.expectRevert(PriceOracle.Unauthorized.selector);
        oracle.updatePrice("bitcoin", BTC_PRICE, block.timestamp);
    }

    function test_InsufficientSignaturesRejected() public {
        bytes32 messageId = keccak256("price-update-insig");
        uint256 timestamp = block.timestamp;

        bytes memory payload = abi.encodeWithSelector(
            oracle.updatePrice.selector,
            "bitcoin",
            BTC_PRICE,
            timestamp
        );

        SequencerSignature memory sig = SequencerSignature({
            signature: new bytes(65),
            submittedAt: block.timestamp
        });

        vm.prank(sequencer);
        bridge.initializeMessage(messageId, address(oracle), payload, sig, 0);

        // Only submit 1 signature (threshold is 2)
        submitValidatorSignatures(messageId, 1);

        // Should fail due to insufficient signatures
        vm.expectRevert();
        bridge.handleMessage(messageId);
    }

    /*//////////////////////////////////////////////////////////////
                        VALIDATION TESTS
    //////////////////////////////////////////////////////////////*/

    function test_RejectZeroPrice() public {
        bytes32 messageId = keccak256("price-update-zero");

        bytes memory payload = abi.encodeWithSelector(
            oracle.updatePrice.selector,
            "bitcoin",
            0, // Zero price
            block.timestamp
        );

        SequencerSignature memory sig = SequencerSignature({
            signature: new bytes(65),
            submittedAt: block.timestamp
        });

        vm.prank(sequencer);
        bridge.initializeMessage(messageId, address(oracle), payload, sig, 0);
        submitValidatorSignatures(messageId, 2);

        // Should fail due to zero price
        vm.expectRevert();
        bridge.handleMessage(messageId);
    }

    function test_RejectFutureTimestamp() public {
        bytes32 messageId = keccak256("price-update-future");

        bytes memory payload = abi.encodeWithSelector(
            oracle.updatePrice.selector,
            "bitcoin",
            BTC_PRICE,
            block.timestamp + 600 // 10 minutes in future (> 5 min tolerance)
        );

        SequencerSignature memory sig = SequencerSignature({
            signature: new bytes(65),
            submittedAt: block.timestamp
        });

        vm.prank(sequencer);
        bridge.initializeMessage(messageId, address(oracle), payload, sig, 0);
        submitValidatorSignatures(messageId, 2);

        // Should fail due to future timestamp
        vm.expectRevert();
        bridge.handleMessage(messageId);
    }

    /*//////////////////////////////////////////////////////////////
                        BATCH UPDATE SIMULATION
    //////////////////////////////////////////////////////////////*/

    function test_BatchPriceUpdates() public {
        uint256 timestamp = block.timestamp;
        SequencerSignature memory sig = SequencerSignature({
            signature: new bytes(65),
            submittedAt: block.timestamp
        });

        string[5] memory assets;
        assets[0] = "bitcoin";
        assets[1] = "ethereum";
        assets[2] = "solana";
        assets[3] = "cardano";
        assets[4] = "polkadot";

        uint256[5] memory assetPrices;
        assetPrices[0] = 67196_645000000000000000;  // BTC
        assetPrices[1] = 3514_490000000000000000;   // ETH
        assetPrices[2] = 145_230000000000000000;    // SOL
        assetPrices[3] = 890000000000000000;        // ADA ($0.89)
        assetPrices[4] = 7_520000000000000000;      // DOT

        for (uint256 i = 0; i < 5; i++) {
            bytes32 messageId = keccak256(abi.encodePacked("batch-price-", i));
            bytes memory payload = abi.encodeWithSelector(
                oracle.updatePrice.selector,
                assets[i],
                assetPrices[i],
                timestamp
            );

            vm.prank(sequencer);
            bridge.initializeMessage(messageId, address(oracle), payload, sig, 0);
            submitValidatorSignatures(messageId, 2);
            bridge.handleMessage(messageId);
        }

        // Verify all prices
        for (uint256 i = 0; i < 5; i++) {
            (uint256 price,,) = oracle.getPrice(assets[i]);
            assertEq(price, assetPrices[i], string.concat("Price mismatch for ", assets[i]));
        }

        assertEq(oracle.assetCount(), 5, "Should track 5 assets");
    }

    /*//////////////////////////////////////////////////////////////
                        INTEGRATION TEST - FULL FLOW
    //////////////////////////////////////////////////////////////*/

    function test_FullPriceOracleWorkflow() public {
        // This test simulates the full flow:
        // 1. Price oracle fetches prices from multiple sources
        // 2. Compares prices (simulated by consistent mock data)
        // 3. Pushes validated price to Bridge
        // 4. Bridge validates and executes update on PriceOracle contract

        uint256 timestamp = block.timestamp;

        // Simulated price from mock sources (consistent within 1% = 100 bps)
        // Mock source 1: $67,196.31
        // Mock source 2: $67,196.98
        // Average: ~$67,196.645
        uint256 avgPriceScaled = 67196_645000000000000000;

        bytes32 messageId = keccak256(abi.encodePacked("validated-price-", timestamp));
        bytes memory payload = abi.encodeWithSelector(
            oracle.updatePrice.selector,
            "bitcoin",
            avgPriceScaled,
            timestamp
        );

        SequencerSignature memory sig = SequencerSignature({
            signature: new bytes(65),
            submittedAt: block.timestamp
        });

        // Sequencer initializes the message
        vm.prank(sequencer);
        bridge.initializeMessage(messageId, address(oracle), payload, sig, 0);

        // Validators sign the message (2 of 3 threshold)
        submitValidatorSignatures(messageId, 2);

        // Execute the message
        bridge.handleMessage(messageId);

        // Verify the price is now on-chain
        (uint256 price, uint256 ts, uint256 blockNum) = oracle.getPrice("bitcoin");
        assertEq(price, avgPriceScaled, "On-chain price should match validated average");
        assertEq(ts, timestamp, "Timestamp should be preserved");
        assertGt(blockNum, 0, "Block number should be set");

        // Verify message is completed
        assertTrue(bridge.isMessageCompleted(messageId), "Message should be completed");
    }

    receive() external payable {}
}
