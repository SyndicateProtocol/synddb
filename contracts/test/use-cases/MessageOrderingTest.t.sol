// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {UseCaseBaseTest} from "./base/UseCaseBaseTest.sol";
import {Bridge} from "src/Bridge.sol";
import {MessageOrderingModule} from "src/modules/MessageOrderingModule.sol";
import {ValidatorSignatureThresholdModule} from "src/modules/ValidatorSignatureThresholdModule.sol";
import {SequencerSignature} from "src/types/DataTypes.sol";
import {WETH9} from "./mocks/WETH9.sol";

/**
 * @title MessageOrderingTest
 * @notice Demonstrates how to use MessageOrderingModule to enforce sequential message processing
 * @dev This use case shows:
 *      1. Setting up a bridge with message ordering enforcement
 *      2. Processing messages in the correct order
 *      3. Preventing out-of-order execution
 *      4. Handling multiple senders with independent nonce sequences
 */
contract MessageOrderingTest is UseCaseBaseTest {
    Bridge public bridge;
    WETH9 public weth;
    MessageOrderingModule public orderingModule;
    ValidatorSignatureThresholdModule public validatorModule;

    address public sequencer;
    address public user1;
    address public user2;
    address public receiver;

    function setUp() public {
        sequencer = vm.addr(sequencerPrivateKey);
        user1 = makeAddr("user1");
        user2 = makeAddr("user2");
        receiver = makeAddr("receiver");

        (bridge, weth) = createBridgeWithWETH(address(this), sequencer);
        validatorModule = setupBridgeWithValidators(bridge);

        // Deploy and configure ordering module
        orderingModule = new MessageOrderingModule(address(this));
        bridge.addPreModule(address(orderingModule));

        vm.deal(sequencer, type(uint128).max);
        vm.deal(user1, type(uint128).max);
        vm.deal(user2, type(uint128).max);
    }

    function _submitValidatorSignatures(bytes32 messageId) internal {
        submitValidatorSignatures(bridge, messageId);
    }

    /*//////////////////////////////////////////////////////////////
                        ORDERED EXECUTION TESTS
    //////////////////////////////////////////////////////////////*/

    function test_OrderedExecution_SequentialMessages() public {
        // Create identifier for user1 on chain 1
        uint256 chainId = 1;
        bytes32 identifier = keccak256(abi.encodePacked(chainId, user1));

        // Process 3 sequential messages
        for (uint256 nonce = 0; nonce < 3; nonce++) {
            bytes32 messageId = keccak256(abi.encodePacked("msg", nonce));
            bytes memory payload = "";
            SequencerSignature memory sig = createSequencerSignature(messageId, receiver, payload, 0);

            // Validate nonce first
            orderingModule.validateNonce(identifier, nonce);

            // Initialize message
            vm.prank(sequencer);
            bridge.initializeMessage(messageId, receiver, payload, sig, 0);

            // Submit validator signatures
            _submitValidatorSignatures(messageId);

            // Handle message
            bridge.handleMessage(messageId);

            assertTrue(bridge.isMessageCompleted(messageId));
        }

        // Verify expected nonce advanced
        assertEq(orderingModule.getExpectedNonce(identifier), 3);
    }

    function test_OrderedExecution_OutOfOrderReverts() public {
        uint256 chainId = 1;
        bytes32 identifier = keccak256(abi.encodePacked(chainId, user1));

        // Try to validate nonce 1 when expecting 0
        vm.expectRevert(abi.encodeWithSelector(MessageOrderingModule.InvalidNonce.selector, identifier, 0, 1));
        orderingModule.validateNonce(identifier, 1);
    }

    function test_OrderedExecution_MultipleUsers() public {
        // Different users maintain independent nonce sequences
        uint256 chainId = 1;
        bytes32 id1 = keccak256(abi.encodePacked(chainId, user1));
        bytes32 id2 = keccak256(abi.encodePacked(chainId, user2));

        // User1 sends 2 messages
        for (uint256 nonce = 0; nonce < 2; nonce++) {
            orderingModule.validateNonce(id1, nonce);

            bytes32 messageId = keccak256(abi.encodePacked("user1-msg", nonce));
            bytes memory payload = "";
            SequencerSignature memory sig = createSequencerSignature(messageId, receiver, payload, 0);

            vm.prank(sequencer);
            bridge.initializeMessage(messageId, receiver, payload, sig, 0);
            _submitValidatorSignatures(messageId);
            bridge.handleMessage(messageId);
        }

        // User2 sends 3 messages (independent sequence)
        for (uint256 nonce = 0; nonce < 3; nonce++) {
            orderingModule.validateNonce(id2, nonce);

            bytes32 messageId = keccak256(abi.encodePacked("user2-msg", nonce));
            bytes memory payload = "";
            SequencerSignature memory sig = createSequencerSignature(messageId, receiver, payload, 0);

            vm.prank(sequencer);
            bridge.initializeMessage(messageId, receiver, payload, sig, 0);
            _submitValidatorSignatures(messageId);
            bridge.handleMessage(messageId);
        }

        // Verify independent nonce tracking
        assertEq(orderingModule.getExpectedNonce(id1), 2);
        assertEq(orderingModule.getExpectedNonce(id2), 3);
    }

    function test_OrderedExecution_CrossChainSeparation() public {
        // Same user on different chains should have independent nonces
        uint256 chain1 = 1;
        uint256 chain2 = 2;
        bytes32 id1 = keccak256(abi.encodePacked(chain1, user1));
        bytes32 id2 = keccak256(abi.encodePacked(chain2, user1));

        // Chain 1: Process 2 messages
        for (uint256 nonce = 0; nonce < 2; nonce++) {
            orderingModule.validateNonce(id1, nonce);
        }

        // Chain 2: Process 4 messages
        for (uint256 nonce = 0; nonce < 4; nonce++) {
            orderingModule.validateNonce(id2, nonce);
        }

        assertEq(orderingModule.getExpectedNonce(id1), 2);
        assertEq(orderingModule.getExpectedNonce(id2), 4);
    }

    /*//////////////////////////////////////////////////////////////
                        EMERGENCY SCENARIOS
    //////////////////////////////////////////////////////////////*/

    function test_EmergencyNonceReset() public {
        uint256 chainId = 1;
        bytes32 identifier = keccak256(abi.encodePacked(chainId, user1));

        // Process some messages
        for (uint256 nonce = 0; nonce < 5; nonce++) {
            orderingModule.validateNonce(identifier, nonce);
        }

        assertEq(orderingModule.getExpectedNonce(identifier), 5);

        // Emergency scenario: Need to reset nonce
        orderingModule.resetNonce(identifier, 10);

        // Should now expect nonce 10
        assertEq(orderingModule.getExpectedNonce(identifier), 10);

        // Can continue from new nonce
        orderingModule.validateNonce(identifier, 10);
        assertEq(orderingModule.getExpectedNonce(identifier), 11);
    }

    function test_BatchNonceReset_Migration() public {
        // Simulate a migration scenario where multiple accounts need nonce reset
        bytes32[] memory identifiers = new bytes32[](3);
        identifiers[0] = keccak256(abi.encodePacked(uint256(1), user1));
        identifiers[1] = keccak256(abi.encodePacked(uint256(1), user2));
        identifiers[2] = keccak256(abi.encodePacked(uint256(2), user1));

        uint256[] memory newNonces = new uint256[](3);
        newNonces[0] = 100;
        newNonces[1] = 200;
        newNonces[2] = 300;

        orderingModule.batchResetNonces(identifiers, newNonces);

        assertEq(orderingModule.getExpectedNonce(identifiers[0]), 100);
        assertEq(orderingModule.getExpectedNonce(identifiers[1]), 200);
        assertEq(orderingModule.getExpectedNonce(identifiers[2]), 300);
    }

    /*//////////////////////////////////////////////////////////////
                        INTEGRATION SCENARIOS
    //////////////////////////////////////////////////////////////*/

    function test_FullFlow_OrderedCrossChainSwaps() public {
        // Simulate a DEX where swap orders must be processed in order
        uint256 sourceChain = 1;
        address trader = user1;
        bytes32 identifier = keccak256(abi.encodePacked(sourceChain, trader));

        // Trader submits 3 sequential swap orders
        string[] memory swaps = new string[](3);
        swaps[0] = "swapExactTokensForTokens(100, 95, [USDC, ETH], trader, deadline)";
        swaps[1] = "swapExactETHForTokens(1, 3000, [ETH, USDC], trader, deadline)";
        swaps[2] = "addLiquidity(USDC, ETH, 1000, 1, 0, 0, trader, deadline)";

        for (uint256 nonce = 0; nonce < swaps.length; nonce++) {
            // Validate ordering
            orderingModule.validateNonce(identifier, nonce);

            // Create message
            bytes32 messageId = keccak256(abi.encodePacked("swap", nonce));
            bytes memory payload = abi.encodeWithSignature(swaps[nonce]);
            SequencerSignature memory sig = createSequencerSignature(messageId, receiver, payload, 0);

            // Initialize and execute
            vm.prank(sequencer);
            bridge.initializeMessage(messageId, receiver, payload, sig, 0);
            _submitValidatorSignatures(messageId);
            bridge.handleMessage(messageId);

            assertTrue(bridge.isMessageCompleted(messageId));
        }

        // Verify all swaps processed in order
        assertEq(orderingModule.getExpectedNonce(identifier), 3);
    }
}
