// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {Test} from "forge-std/Test.sol";
import {Bridge} from "src/Bridge.sol";
import {SequencerSignature} from "src/types/DataTypes.sol";
import {ValidatorSignatureThresholdModule} from "src/modules/ValidatorSignatureThresholdModule.sol";
import {WETH9} from "./use-cases/mocks/WETH9.sol";
import {MessageHashUtils} from "@openzeppelin/contracts/utils/cryptography/MessageHashUtils.sol";

/**
 * @title BridgeTest
 * @notice Comprehensive unit and fuzz tests for the Bridge contract
 * @dev Tests core Bridge functionality including ETH handling, message lifecycle, and edge cases
 */
contract BridgeTest is Test {
    Bridge public bridge;
    WETH9 public weth;
    ValidatorSignatureThresholdModule public validatorModule;

    address public admin;
    address public sequencer;
    address public user;
    address public receiver;

    // Validators
    uint256[] public validatorPrivateKeys;
    address[] public validators;

    event MessageInitialized(bytes32 indexed messageId, bytes payload);
    event MessageHandled(bytes32 indexed messageId, bool success);
    event NativeTokenWrapped(address indexed sender, uint256 amount);
    event NativeTokenUnwrapped(uint256 amount, address indexed target);

    function setUp() public {
        admin = address(this);
        sequencer = makeAddr("sequencer");
        user = makeAddr("user");
        receiver = makeAddr("receiver");

        weth = new WETH9();
        bridge = new Bridge(admin, address(weth));

        setupValidators(3);
        validatorModule = new ValidatorSignatureThresholdModule(address(bridge), validators, 2);

        bridge.grantRole(bridge.SEQUENCER_ROLE(), sequencer);
        bridge.addPreModule(address(validatorModule));

        vm.deal(sequencer, type(uint128).max);
        vm.deal(user, type(uint128).max);
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

    function signMessage(bytes32 messageId, uint256 validatorPrivateKey) internal pure returns (bytes memory) {
        bytes32 ethSignedMessageHash = MessageHashUtils.toEthSignedMessageHash(messageId);
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(validatorPrivateKey, ethSignedMessageHash);
        return abi.encodePacked(r, s, v);
    }

    function submitValidatorSignatures(bytes32 messageId, uint256 signatureCount) internal {
        require(signatureCount <= validatorPrivateKeys.length, "Not enough validators");
        for (uint256 i = 0; i < signatureCount; i++) {
            bytes memory sig = signMessage(messageId, validatorPrivateKeys[i]);
            bridge.signMessageWithSignature(messageId, sig);
        }
    }

    function submitValidatorSignatures(bytes32 messageId) internal {
        submitValidatorSignatures(messageId, 2);
    }

    /*//////////////////////////////////////////////////////////////
                        BASIC MESSAGE LIFECYCLE
    //////////////////////////////////////////////////////////////*/

    function test_InitializeMessage_Basic() public {
        bytes32 messageId = keccak256("test-message");
        bytes memory payload = abi.encodeWithSignature("transfer(address,uint256)", receiver, 100);
        SequencerSignature memory sig = SequencerSignature({signature: new bytes(65), submittedAt: block.timestamp});

        vm.prank(sequencer);
        vm.expectEmit(true, false, false, true);
        emit MessageInitialized(messageId, payload);
        bridge.initializeMessage{value: 0}(messageId, receiver, payload, sig, 0);

        assertTrue(bridge.isMessageInitialized(messageId));
        assertFalse(bridge.isMessageCompleted(messageId));
    }

    function test_InitializeMessage_RevertsIfAlreadyInitialized() public {
        bytes32 messageId = keccak256("duplicate-message");
        bytes memory payload = "";
        SequencerSignature memory sig = SequencerSignature({signature: new bytes(65), submittedAt: block.timestamp});

        vm.startPrank(sequencer);
        bridge.initializeMessage{value: 0}(messageId, receiver, payload, sig, 0);

        vm.expectRevert(abi.encodeWithSelector(Bridge.MessageAlreadyInitialized.selector, messageId));
        bridge.initializeMessage{value: 0}(messageId, receiver, payload, sig, 0);
        vm.stopPrank();
    }

    function test_InitializeMessage_OnlySequencer() public {
        bytes32 messageId = keccak256("unauthorized");
        bytes memory payload = "";
        SequencerSignature memory sig = SequencerSignature({signature: new bytes(65), submittedAt: block.timestamp});

        vm.prank(user);
        vm.expectRevert();
        bridge.initializeMessage{value: 0}(messageId, receiver, payload, sig, 0);
    }

    function test_HandleMessage_NotInitialized_Reverts() public {
        bytes32 messageId = keccak256("not-initialized");

        vm.expectRevert(abi.encodeWithSelector(Bridge.MessageNotInitialized.selector, messageId));
        bridge.handleMessage(messageId);
    }

    function test_HandleMessage_AlreadyHandled_Reverts() public {
        bytes32 messageId = keccak256("already-handled");
        bytes memory payload = "";
        SequencerSignature memory sig = SequencerSignature({signature: new bytes(65), submittedAt: block.timestamp});

        vm.prank(sequencer);
        bridge.initializeMessage{value: 0}(messageId, receiver, payload, sig, 0);

        submitValidatorSignatures(messageId);

        bridge.handleMessage(messageId);
        assertTrue(bridge.isMessageCompleted(messageId));

        vm.expectRevert(abi.encodeWithSelector(Bridge.MessageAlreadyHandled.selector, messageId));
        bridge.handleMessage(messageId);
    }

    function test_HandleMessage_CompletesSuccessfully() public {
        bytes32 messageId = keccak256("complete-message");
        bytes memory payload = "";
        SequencerSignature memory sig = SequencerSignature({signature: new bytes(65), submittedAt: block.timestamp});

        vm.prank(sequencer);
        bridge.initializeMessage{value: 0}(messageId, receiver, payload, sig, 0);

        submitValidatorSignatures(messageId);

        vm.expectEmit(true, false, false, true);
        emit MessageHandled(messageId, true);
        bridge.handleMessage(messageId);

        assertTrue(bridge.isMessageCompleted(messageId));
        assertTrue(bridge.isMessageHandled(messageId));
    }

    /*//////////////////////////////////////////////////////////////
                    ETH AMOUNT VALIDATION TESTS
    //////////////////////////////////////////////////////////////*/

    function test_InitializeMessage_InvalidETHAmount_Reverts() public {
        bytes32 messageId = keccak256("invalid-eth");
        bytes memory payload = "";
        SequencerSignature memory sig = SequencerSignature({signature: new bytes(65), submittedAt: block.timestamp});

        vm.prank(sequencer);
        vm.expectRevert(abi.encodeWithSelector(Bridge.InvalidETHAmount.selector, 1 ether, 0.5 ether));
        bridge.initializeMessage{value: 1 ether}(messageId, receiver, payload, sig, 0.5 ether);
    }

    function test_InitializeMessage_ZeroEthAmountWithValue_Reverts() public {
        bytes32 messageId = keccak256("zero-mismatch");
        bytes memory payload = "";
        SequencerSignature memory sig = SequencerSignature({signature: new bytes(65), submittedAt: block.timestamp});

        vm.prank(sequencer);
        vm.expectRevert(abi.encodeWithSelector(Bridge.InvalidETHAmount.selector, 0.5 ether, 0));
        bridge.initializeMessage{value: 0.5 ether}(messageId, receiver, payload, sig, 0);
    }

    function test_InitializeMessage_MatchingETH_Succeeds() public {
        bytes32 messageId = keccak256("matching-eth");
        bytes memory payload = "";
        SequencerSignature memory sig = SequencerSignature({signature: new bytes(65), submittedAt: block.timestamp});
        uint256 amount = 1 ether;

        uint256 bridgeWethBefore = weth.balanceOf(address(bridge));

        vm.prank(sequencer);
        vm.expectEmit(true, false, false, true);
        emit NativeTokenWrapped(sequencer, amount);
        bridge.initializeMessage{value: amount}(messageId, receiver, payload, sig, amount);

        assertEq(weth.balanceOf(address(bridge)), bridgeWethBefore + amount);
        assertTrue(bridge.isMessageInitialized(messageId));
    }

    function test_InitializeMessage_BothZero_Succeeds() public {
        bytes32 messageId = keccak256("both-zero");
        bytes memory payload = "";
        SequencerSignature memory sig = SequencerSignature({signature: new bytes(65), submittedAt: block.timestamp});

        vm.prank(sequencer);
        bridge.initializeMessage{value: 0}(messageId, receiver, payload, sig, 0);

        assertTrue(bridge.isMessageInitialized(messageId));
    }

    /*//////////////////////////////////////////////////////////////
                    ETH HANDLING IN HANDLEMESSAGE
    //////////////////////////////////////////////////////////////*/

    function test_HandleMessage_UnwrapsAndSendsETH() public {
        bytes32 messageId = keccak256("unwrap-send");
        bytes memory payload = "";
        SequencerSignature memory sig = SequencerSignature({signature: new bytes(65), submittedAt: block.timestamp});
        uint256 amount = 1 ether;

        vm.prank(sequencer);
        bridge.initializeMessage{value: amount}(messageId, receiver, payload, sig, amount);

        submitValidatorSignatures(messageId);

        uint256 receiverBalanceBefore = receiver.balance;
        uint256 bridgeWethBefore = weth.balanceOf(address(bridge));

        vm.expectEmit(false, true, false, true);
        emit NativeTokenUnwrapped(amount, receiver);
        bridge.handleMessage(messageId);

        assertEq(receiver.balance, receiverBalanceBefore + amount);
        assertEq(weth.balanceOf(address(bridge)), bridgeWethBefore - amount);
    }

    function test_HandleMessage_InsufficientWETH_Reverts() public {
        bytes32 messageId = keccak256("insufficient-weth");
        bytes memory payload = "";
        SequencerSignature memory sig = SequencerSignature({signature: new bytes(65), submittedAt: block.timestamp});
        uint256 amount = 1 ether;

        vm.prank(sequencer);
        bridge.initializeMessage{value: amount}(messageId, receiver, payload, sig, amount);

        // Steal the WETH
        vm.prank(address(bridge));
        weth.transfer(user, amount);

        submitValidatorSignatures(messageId);

        vm.expectRevert(abi.encodeWithSelector(Bridge.InsufficientWETHBalance.selector, amount, 0));
        bridge.handleMessage(messageId);
    }

    function test_HandleMessage_PartialWETH_Reverts() public {
        bytes32 messageId = keccak256("partial-weth");
        bytes memory payload = "";
        SequencerSignature memory sig = SequencerSignature({signature: new bytes(65), submittedAt: block.timestamp});
        uint256 required = 1 ether;
        uint256 available = 0.5 ether;

        vm.prank(sequencer);
        bridge.initializeMessage{value: required}(messageId, receiver, payload, sig, required);

        vm.prank(address(bridge));
        weth.transfer(user, available);

        submitValidatorSignatures(messageId);

        vm.expectRevert(abi.encodeWithSelector(Bridge.InsufficientWETHBalance.selector, required, available));
        bridge.handleMessage(messageId);
    }

    function test_HandleMessage_ZeroETH_NoUnwrap() public {
        // Give bridge some existing WETH
        vm.prank(user);
        weth.deposit{value: 10 ether}();
        vm.prank(user);
        weth.transfer(address(bridge), 10 ether);

        bytes32 messageId = keccak256("zero-eth");
        bytes memory payload = "";
        SequencerSignature memory sig = SequencerSignature({signature: new bytes(65), submittedAt: block.timestamp});

        uint256 bridgeWethBefore = weth.balanceOf(address(bridge));

        vm.prank(sequencer);
        bridge.initializeMessage{value: 0}(messageId, receiver, payload, sig, 0);

        submitValidatorSignatures(messageId);

        bridge.handleMessage(messageId);

        // WETH should remain unchanged
        assertEq(weth.balanceOf(address(bridge)), bridgeWethBefore);
        assertTrue(bridge.isMessageCompleted(messageId));
    }

    function test_HandleMessage_CallFailure_Reverts() public {
        RevertingContract reverter = new RevertingContract();
        bytes32 messageId = keccak256("call-fail");
        bytes memory payload = abi.encodeWithSignature("alwaysReverts()");
        SequencerSignature memory sig = SequencerSignature({signature: new bytes(65), submittedAt: block.timestamp});
        uint256 amount = 1 ether;

        vm.prank(sequencer);
        bridge.initializeMessage{value: amount}(messageId, address(reverter), payload, sig, amount);

        submitValidatorSignatures(messageId);

        vm.expectRevert();
        bridge.handleMessage(messageId);
    }

    /*//////////////////////////////////////////////////////////////
                        RECEIVE FUNCTION TESTS
    //////////////////////////////////////////////////////////////*/

    function test_Receive_WrapsETH() public {
        uint256 amount = 5 ether;
        uint256 bridgeWethBefore = weth.balanceOf(address(bridge));

        vm.prank(user);
        vm.expectEmit(true, false, false, true);
        emit NativeTokenWrapped(user, amount);
        (bool success,) = address(bridge).call{value: amount}("");
        assertTrue(success);

        assertEq(weth.balanceOf(address(bridge)), bridgeWethBefore + amount);
    }

    function test_Receive_FromWETH_NoReWrap() public {
        // Give bridge WETH
        vm.prank(user);
        weth.deposit{value: 10 ether}();
        vm.prank(user);
        weth.transfer(address(bridge), 10 ether);

        uint256 bridgeWethBefore = weth.balanceOf(address(bridge));
        uint256 bridgeEthBefore = address(bridge).balance;
        uint256 withdrawAmount = 5 ether;

        // Unwrap WETH
        vm.prank(address(bridge));
        weth.withdraw(withdrawAmount);

        // WETH should decrease (not re-wrap)
        assertEq(weth.balanceOf(address(bridge)), bridgeWethBefore - withdrawAmount);
        assertEq(address(bridge).balance, bridgeEthBefore + withdrawAmount);
    }

    /*//////////////////////////////////////////////////////////////
                INITIALIZEANDHANDLEMESSAGE TESTS
    //////////////////////////////////////////////////////////////*/

    function test_InitializeAndHandleMessage_WithETH() public {
        bytes32 messageId = keccak256("init-handle-eth");
        bytes memory payload = "";
        SequencerSignature memory sig = SequencerSignature({signature: new bytes(65), submittedAt: block.timestamp});
        uint256 amount = 1 ether;

        bytes[] memory validatorSigs = new bytes[](2);
        for (uint256 i = 0; i < 2; i++) {
            validatorSigs[i] = signMessage(messageId, validatorPrivateKeys[i]);
        }

        uint256 receiverBalanceBefore = receiver.balance;

        vm.prank(sequencer);
        bridge.initializeAndHandleMessage{value: amount}(messageId, receiver, payload, sig, validatorSigs, amount);

        assertEq(receiver.balance, receiverBalanceBefore + amount);
        assertTrue(bridge.isMessageCompleted(messageId));
    }

    function test_InitializeAndHandleMessage_InvalidETH_Reverts() public {
        bytes32 messageId = keccak256("init-handle-invalid");
        bytes memory payload = "";
        SequencerSignature memory sig = SequencerSignature({signature: new bytes(65), submittedAt: block.timestamp});
        bytes[] memory validatorSigs = new bytes[](0);

        vm.prank(sequencer);
        vm.expectRevert(abi.encodeWithSelector(Bridge.InvalidETHAmount.selector, 1 ether, 0.5 ether));
        bridge.initializeAndHandleMessage{value: 1 ether}(messageId, receiver, payload, sig, validatorSigs, 0.5 ether);
    }

    /*//////////////////////////////////////////////////////////////
                    BATCH OPERATIONS TESTS
    //////////////////////////////////////////////////////////////*/

    function test_BatchInitializeMessage_WithETH() public {
        uint256 batchSize = 3;
        bytes32[] memory messageIds = new bytes32[](batchSize);
        address[] memory targetAddresses = new address[](batchSize);
        bytes[] memory payloads = new bytes[](batchSize);
        SequencerSignature[] memory sigs = new SequencerSignature[](batchSize);
        uint256[] memory ethAmounts = new uint256[](batchSize);

        uint256 totalEth = 0;

        for (uint256 i = 0; i < batchSize; i++) {
            messageIds[i] = keccak256(abi.encodePacked("batch", i));
            targetAddresses[i] = receiver;
            payloads[i] = "";
            sigs[i] = SequencerSignature({signature: new bytes(65), submittedAt: block.timestamp});
            ethAmounts[i] = (i + 1) * 0.1 ether;
            totalEth += ethAmounts[i];
        }

        uint256 bridgeWethBefore = weth.balanceOf(address(bridge));

        vm.prank(sequencer);
        bridge.batchInitializeMessage{value: totalEth}(messageIds, targetAddresses, payloads, sigs, ethAmounts);

        for (uint256 i = 0; i < batchSize; i++) {
            assertTrue(bridge.isMessageInitialized(messageIds[i]));
        }

        assertEq(weth.balanceOf(address(bridge)), bridgeWethBefore + totalEth);
    }

    function test_BatchInitializeMessage_MismatchedTotal_Reverts() public {
        bytes32[] memory messageIds = new bytes32[](2);
        address[] memory targetAddresses = new address[](2);
        bytes[] memory payloads = new bytes[](2);
        SequencerSignature[] memory sigs = new SequencerSignature[](2);
        uint256[] memory ethAmounts = new uint256[](2);

        messageIds[0] = keccak256("batch1");
        messageIds[1] = keccak256("batch2");
        targetAddresses[0] = receiver;
        targetAddresses[1] = receiver;
        payloads[0] = "";
        payloads[1] = "";
        sigs[0] = SequencerSignature({signature: new bytes(65), submittedAt: block.timestamp});
        sigs[1] = SequencerSignature({signature: new bytes(65), submittedAt: block.timestamp});
        ethAmounts[0] = 0.5 ether;
        ethAmounts[1] = 0.5 ether;

        vm.prank(sequencer);
        vm.expectRevert(abi.encodeWithSelector(Bridge.InvalidETHAmount.selector, 0.8 ether, 1 ether));
        bridge.batchInitializeMessage{value: 0.8 ether}(messageIds, targetAddresses, payloads, sigs, ethAmounts);
    }

    function test_BatchInitializeMessage_ArrayLengthMismatch_Reverts() public {
        bytes32[] memory messageIds = new bytes32[](2);
        address[] memory targetAddresses = new address[](1); // Mismatched length
        bytes[] memory payloads = new bytes[](2);
        SequencerSignature[] memory sigs = new SequencerSignature[](2);
        uint256[] memory ethAmounts = new uint256[](2);

        vm.prank(sequencer);
        vm.expectRevert(Bridge.ArrayLengthMismatch.selector);
        bridge.batchInitializeMessage{value: 0}(messageIds, targetAddresses, payloads, sigs, ethAmounts);
    }

    function test_BatchHandleMessage() public {
        uint256 batchSize = 3;
        bytes32[] memory messageIds = new bytes32[](batchSize);

        for (uint256 i = 0; i < batchSize; i++) {
            messageIds[i] = keccak256(abi.encodePacked("batch-handle", i));
            bytes memory payload = "";
            SequencerSignature memory sig = SequencerSignature({signature: new bytes(65), submittedAt: block.timestamp});

            vm.prank(sequencer);
            bridge.initializeMessage{value: 0}(messageIds[i], receiver, payload, sig, 0);

            submitValidatorSignatures(messageIds[i]);
        }

        bridge.batchHandleMessage(messageIds);

        for (uint256 i = 0; i < batchSize; i++) {
            assertTrue(bridge.isMessageCompleted(messageIds[i]));
        }
    }

    /*//////////////////////////////////////////////////////////////
                        FUZZING TESTS
    //////////////////////////////////////////////////////////////*/

    function testFuzz_InitializeMessage_MismatchedValue_AlwaysReverts(uint256 msgValue, uint256 ethAmount) public {
        vm.assume(msgValue != ethAmount);
        msgValue = bound(msgValue, 0, type(uint128).max);
        ethAmount = bound(ethAmount, 0, type(uint128).max);

        bytes32 messageId = keccak256(abi.encodePacked("fuzz-mismatch", msgValue, ethAmount));
        bytes memory payload = "";
        SequencerSignature memory sig = SequencerSignature({signature: new bytes(65), submittedAt: block.timestamp});

        vm.prank(sequencer);
        vm.expectRevert(abi.encodeWithSelector(Bridge.InvalidETHAmount.selector, msgValue, ethAmount));
        bridge.initializeMessage{value: msgValue}(messageId, receiver, payload, sig, ethAmount);
    }

    function testFuzz_InitializeMessage_MatchingValue_AlwaysSucceeds(uint256 ethAmount) public {
        ethAmount = bound(ethAmount, 0, type(uint96).max);

        bytes32 messageId = keccak256(abi.encodePacked("fuzz-match", ethAmount));
        bytes memory payload = "";
        SequencerSignature memory sig = SequencerSignature({signature: new bytes(65), submittedAt: block.timestamp});

        uint256 bridgeWethBefore = weth.balanceOf(address(bridge));

        vm.prank(sequencer);
        bridge.initializeMessage{value: ethAmount}(messageId, receiver, payload, sig, ethAmount);

        assertEq(weth.balanceOf(address(bridge)), bridgeWethBefore + ethAmount);
        assertTrue(bridge.isMessageInitialized(messageId));
    }

    function testFuzz_HandleMessage_CorrectlyUnwrapsAndSendsETH(uint256 ethAmount) public {
        ethAmount = bound(ethAmount, 1 wei, type(uint96).max);

        bytes32 messageId = keccak256(abi.encodePacked("fuzz-handle", ethAmount));
        bytes memory payload = "";
        SequencerSignature memory sig = SequencerSignature({signature: new bytes(65), submittedAt: block.timestamp});

        vm.prank(sequencer);
        bridge.initializeMessage{value: ethAmount}(messageId, receiver, payload, sig, ethAmount);

        submitValidatorSignatures(messageId);

        uint256 receiverBalanceBefore = receiver.balance;
        uint256 bridgeWethBefore = weth.balanceOf(address(bridge));

        bridge.handleMessage(messageId);

        assertEq(receiver.balance, receiverBalanceBefore + ethAmount);
        assertEq(weth.balanceOf(address(bridge)), bridgeWethBefore - ethAmount);
        assertTrue(bridge.isMessageCompleted(messageId));
    }

    function testFuzz_HandleMessage_InsufficientWETH_AlwaysReverts(uint256 requiredAmount, uint256 stolenAmount)
        public
    {
        requiredAmount = bound(requiredAmount, 1 ether, type(uint96).max);
        stolenAmount = bound(stolenAmount, 1 wei, requiredAmount);
        uint256 remainingAmount = requiredAmount - stolenAmount;

        bytes32 messageId = keccak256(abi.encodePacked("fuzz-insufficient", requiredAmount, stolenAmount));
        bytes memory payload = "";
        SequencerSignature memory sig = SequencerSignature({signature: new bytes(65), submittedAt: block.timestamp});

        vm.prank(sequencer);
        bridge.initializeMessage{value: requiredAmount}(messageId, receiver, payload, sig, requiredAmount);

        vm.prank(address(bridge));
        weth.transfer(user, stolenAmount);

        submitValidatorSignatures(messageId);

        vm.expectRevert(
            abi.encodeWithSelector(Bridge.InsufficientWETHBalance.selector, requiredAmount, remainingAmount)
        );
        bridge.handleMessage(messageId);
    }

    function testFuzz_BatchInitialize_VariousAmounts(uint8 arrayLength, uint256 seed) public {
        arrayLength = uint8(bound(arrayLength, 1, 10));

        bytes32[] memory messageIds = new bytes32[](arrayLength);
        address[] memory targetAddresses = new address[](arrayLength);
        bytes[] memory payloads = new bytes[](arrayLength);
        SequencerSignature[] memory sigs = new SequencerSignature[](arrayLength);
        uint256[] memory ethAmounts = new uint256[](arrayLength);

        uint256 totalEth = 0;

        for (uint256 i = 0; i < arrayLength; i++) {
            messageIds[i] = keccak256(abi.encodePacked("fuzz-batch", seed, i));
            targetAddresses[i] = receiver;
            payloads[i] = "";
            sigs[i] = SequencerSignature({signature: new bytes(65), submittedAt: block.timestamp});
            ethAmounts[i] = bound(uint256(keccak256(abi.encodePacked(seed, i))), 0, 10 ether);
            totalEth += ethAmounts[i];
        }

        vm.assume(totalEth <= type(uint96).max);

        uint256 bridgeWethBefore = weth.balanceOf(address(bridge));

        vm.prank(sequencer);
        bridge.batchInitializeMessage{value: totalEth}(messageIds, targetAddresses, payloads, sigs, ethAmounts);

        for (uint256 i = 0; i < arrayLength; i++) {
            assertTrue(bridge.isMessageInitialized(messageIds[i]));
        }

        assertEq(weth.balanceOf(address(bridge)), bridgeWethBefore + totalEth);
    }

    function testFuzz_CompleteETHFlow(uint96 ethAmount) public {
        vm.assume(ethAmount > 0);

        // User wraps ETH
        vm.prank(user);
        (bool success,) = address(bridge).call{value: ethAmount}("");
        assertTrue(success);
        uint256 bridgeWethAfterWrap = weth.balanceOf(address(bridge));

        // Sequencer initializes message
        bytes32 messageId = keccak256(abi.encodePacked("fuzz-complete-flow", ethAmount));
        bytes memory payload = "";
        SequencerSignature memory sig = SequencerSignature({signature: new bytes(65), submittedAt: block.timestamp});

        vm.prank(sequencer);
        bridge.initializeMessage{value: ethAmount}(messageId, receiver, payload, sig, ethAmount);

        uint256 bridgeWethAfterInit = weth.balanceOf(address(bridge));
        assertEq(bridgeWethAfterInit, bridgeWethAfterWrap + ethAmount);

        // Handle message
        submitValidatorSignatures(messageId);

        uint256 receiverBalanceBefore = receiver.balance;
        bridge.handleMessage(messageId);

        assertEq(receiver.balance, receiverBalanceBefore + ethAmount);
        assertEq(weth.balanceOf(address(bridge)), bridgeWethAfterInit - ethAmount);
    }

    receive() external payable {}
}

contract RevertingContract {
    function alwaysReverts() external payable {
        revert("Always reverts");
    }
}
