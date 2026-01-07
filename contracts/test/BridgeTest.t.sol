// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {Test} from "forge-std/Test.sol";
import {Bridge} from "src/Bridge.sol";
import {SequencerSignature, KeyType} from "src/types/DataTypes.sol";
import {ValidatorSignatureThresholdModule} from "src/modules/ValidatorSignatureThresholdModule.sol";
import {TeeKeyManager} from "src/attestation/TeeKeyManager.sol";
import {MockAttestationVerifier} from "src/attestation/MockAttestationVerifier.sol";
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
    TeeKeyManager public teeKeyManager;
    MockAttestationVerifier public attestationVerifier;

    address public admin;
    address public sequencer;
    uint256 public sequencerPrivateKey;
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
        sequencerPrivateKey = 0xA11CE;
        sequencer = vm.addr(sequencerPrivateKey);
        user = makeAddr("user");
        receiver = makeAddr("receiver");

        weth = new WETH9();

        // Deploy attestation infrastructure
        attestationVerifier = new MockAttestationVerifier();
        teeKeyManager = new TeeKeyManager(attestationVerifier);

        // Deploy bridge first (TeeKeyManager needs bridge address)
        bridge = new Bridge(admin, address(weth), address(teeKeyManager));

        // Set bridge on TeeKeyManager
        teeKeyManager.setBridge(address(bridge));

        // Register sequencer as a valid TEE key through bridge
        bytes memory publicValues = abi.encode(sequencer);
        bridge.registerKey(KeyType.Sequencer, publicValues, "");

        // Setup validators and register them
        setupValidators(3);

        // Deploy validator module with TeeKeyManager reference
        validatorModule = new ValidatorSignatureThresholdModule(address(bridge), address(teeKeyManager), 2);

        // Grant message initializer permission to sequencer
        bridge.setMessageInitializer(sequencer, true);
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

            // Register validator key through bridge
            bytes memory publicValues = abi.encode(validatorAddr);
            bridge.registerKey(KeyType.Validator, publicValues, "");
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

    /// @notice Create a sequencer signature for a message
    function createSequencerSignature(
        bytes32 messageId,
        address targetAddress,
        bytes memory payload,
        uint256 nativeTokenAmount
    ) internal view returns (SequencerSignature memory) {
        bytes32 messageHash = keccak256(
            abi.encodePacked(messageId, targetAddress, keccak256(payload), nativeTokenAmount)
        );
        bytes32 ethSignedHash = MessageHashUtils.toEthSignedMessageHash(messageHash);
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(sequencerPrivateKey, ethSignedHash);
        return SequencerSignature({signature: abi.encodePacked(r, s, v), submittedAt: block.timestamp});
    }

    /*//////////////////////////////////////////////////////////////
                        CONSTRUCTOR TESTS
    //////////////////////////////////////////////////////////////*/

    function test_Constructor_RevertsOnZeroAdminAddress() public {
        // Ownable2Step rejects zero address owner before our check runs
        vm.expectRevert(abi.encodeWithSignature("OwnableInvalidOwner(address)", address(0)));
        new Bridge(address(0), address(weth), address(teeKeyManager));
    }

    function test_Constructor_RevertsOnZeroWrappedNativeTokenAddress() public {
        vm.expectRevert(Bridge.ZeroAddressNotAllowed.selector);
        new Bridge(admin, address(0), address(teeKeyManager));
    }

    function test_Constructor_RevertsOnZeroTeeKeyManagerAddress() public {
        vm.expectRevert(Bridge.ZeroAddressNotAllowed.selector);
        new Bridge(admin, address(weth), address(0));
    }

    function test_Constructor_RevertsOnAllZeroAddresses() public {
        // Ownable2Step rejects zero address owner before our check runs
        vm.expectRevert(abi.encodeWithSignature("OwnableInvalidOwner(address)", address(0)));
        new Bridge(address(0), address(0), address(0));
    }

    /*//////////////////////////////////////////////////////////////
                        BASIC MESSAGE LIFECYCLE
    //////////////////////////////////////////////////////////////*/

    function test_InitializeMessage_Basic() public {
        bytes32 messageId = keccak256("test-message");
        bytes memory payload = abi.encodeWithSignature("transfer(address,uint256)", receiver, 100);
        SequencerSignature memory sig = createSequencerSignature(messageId, receiver, payload, 0);

        vm.prank(sequencer);
        vm.expectEmit(true, false, false, true);
        emit MessageInitialized(messageId, payload);
        bridge.initializeMessage(messageId, receiver, payload, sig, 0);

        assertTrue(bridge.isMessageInitialized(messageId));
        assertFalse(bridge.isMessageCompleted(messageId));
    }

    function test_InitializeMessage_RevertsIfAlreadyInitialized() public {
        bytes32 messageId = keccak256("duplicate-message");
        bytes memory payload = "";
        SequencerSignature memory sig = createSequencerSignature(messageId, receiver, payload, 0);

        vm.startPrank(sequencer);
        bridge.initializeMessage(messageId, receiver, payload, sig, 0);

        vm.expectRevert(abi.encodeWithSelector(Bridge.MessageAlreadyInitialized.selector, messageId));
        bridge.initializeMessage(messageId, receiver, payload, sig, 0);
        vm.stopPrank();
    }

    function test_InitializeMessage_OnlySequencer() public {
        bytes32 messageId = keccak256("unauthorized");
        bytes memory payload = "";
        SequencerSignature memory sig = createSequencerSignature(messageId, receiver, payload, 0);

        vm.prank(user);
        vm.expectRevert();
        bridge.initializeMessage(messageId, receiver, payload, sig, 0);
    }

    function test_InitializeMessage_InvalidSignature_Reverts() public {
        bytes32 messageId = keccak256("invalid-sig");
        bytes memory payload = "";
        // Create a signature with wrong private key (not registered in TeeKeyManager)
        uint256 wrongKey = 0xBAD;
        bytes32 messageHash = keccak256(abi.encodePacked(messageId, receiver, keccak256(payload), uint256(0)));
        bytes32 ethSignedHash = MessageHashUtils.toEthSignedMessageHash(messageHash);
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(wrongKey, ethSignedHash);
        SequencerSignature memory sig =
            SequencerSignature({signature: abi.encodePacked(r, s, v), submittedAt: block.timestamp});

        vm.prank(sequencer);
        vm.expectRevert();
        bridge.initializeMessage(messageId, receiver, payload, sig, 0);
    }

    function test_HandleMessage_NotInitialized_Reverts() public {
        bytes32 messageId = keccak256("not-initialized");

        vm.expectRevert(abi.encodeWithSelector(Bridge.MessageNotInitialized.selector, messageId));
        bridge.handleMessage(messageId);
    }

    function test_HandleMessage_AlreadyHandled_Reverts() public {
        bytes32 messageId = keccak256("already-handled");
        bytes memory payload = "";
        SequencerSignature memory sig = createSequencerSignature(messageId, receiver, payload, 0);

        vm.prank(sequencer);
        bridge.initializeMessage(messageId, receiver, payload, sig, 0);

        submitValidatorSignatures(messageId);

        bridge.handleMessage(messageId);
        assertTrue(bridge.isMessageCompleted(messageId));

        vm.expectRevert(abi.encodeWithSelector(Bridge.MessageAlreadyHandled.selector, messageId));
        bridge.handleMessage(messageId);
    }

    function test_HandleMessage_CompletesSuccessfully() public {
        bytes32 messageId = keccak256("complete-message");
        bytes memory payload = "";
        SequencerSignature memory sig = createSequencerSignature(messageId, receiver, payload, 0);

        vm.prank(sequencer);
        bridge.initializeMessage(messageId, receiver, payload, sig, 0);

        submitValidatorSignatures(messageId);

        vm.expectEmit(true, false, false, true);
        emit MessageHandled(messageId, true);
        bridge.handleMessage(messageId);

        assertTrue(bridge.isMessageCompleted(messageId));
        assertTrue(bridge.isMessageHandled(messageId));
    }

    /*//////////////////////////////////////////////////////////////
                    NATIVE TOKEN AMOUNT TESTS
    //////////////////////////////////////////////////////////////*/

    function test_InitializeMessage_WithNativeTokenAmount_Succeeds() public {
        bytes32 messageId = keccak256("with-native-token");
        bytes memory payload = "";
        uint256 amount = 1 ether;
        SequencerSignature memory sig = createSequencerSignature(messageId, receiver, payload, amount);

        // First deposit ETH to bridge to wrap to WETH
        vm.prank(user);
        (bool success,) = address(bridge).call{value: amount}("");
        assertTrue(success);

        uint256 bridgeWethBefore = weth.balanceOf(address(bridge));

        vm.prank(sequencer);
        bridge.initializeMessage(messageId, receiver, payload, sig, amount);

        // WETH balance should remain the same (no wrapping in initializeMessage)
        assertEq(weth.balanceOf(address(bridge)), bridgeWethBefore);
        assertTrue(bridge.isMessageInitialized(messageId));
    }

    function test_InitializeMessage_ZeroAmount_Succeeds() public {
        bytes32 messageId = keccak256("zero-amount");
        bytes memory payload = "";
        SequencerSignature memory sig = createSequencerSignature(messageId, receiver, payload, 0);

        vm.prank(sequencer);
        bridge.initializeMessage(messageId, receiver, payload, sig, 0);

        assertTrue(bridge.isMessageInitialized(messageId));
    }

    /*//////////////////////////////////////////////////////////////
                    ETH HANDLING IN HANDLEMESSAGE
    //////////////////////////////////////////////////////////////*/

    function test_HandleMessage_UnwrapsAndSendsETH() public {
        bytes32 messageId = keccak256("unwrap-send");
        bytes memory payload = "";
        uint256 amount = 1 ether;
        SequencerSignature memory sig = createSequencerSignature(messageId, receiver, payload, amount);

        // First deposit ETH to bridge (wraps to WETH)
        vm.prank(user);
        (bool success,) = address(bridge).call{value: amount}("");
        assertTrue(success);

        uint256 bridgeWethBefore = weth.balanceOf(address(bridge));

        vm.prank(sequencer);
        bridge.initializeMessage(messageId, receiver, payload, sig, amount);

        submitValidatorSignatures(messageId);

        uint256 receiverBalanceBefore = receiver.balance;

        vm.expectEmit(false, true, false, true);
        emit NativeTokenUnwrapped(amount, receiver);
        bridge.handleMessage(messageId);

        assertEq(receiver.balance, receiverBalanceBefore + amount);
        assertEq(weth.balanceOf(address(bridge)), bridgeWethBefore - amount);
    }

    function test_HandleMessage_InsufficientWrappedNativeToken_Reverts() public {
        bytes32 messageId = keccak256("insufficient-weth");
        bytes memory payload = "";
        uint256 amount = 1 ether;
        SequencerSignature memory sig = createSequencerSignature(messageId, receiver, payload, amount);

        // Deposit ETH to bridge first
        vm.prank(user);
        (bool success,) = address(bridge).call{value: amount}("");
        assertTrue(success);

        vm.prank(sequencer);
        bridge.initializeMessage(messageId, receiver, payload, sig, amount);

        // Steal the WETH
        vm.prank(address(bridge));
        weth.transfer(user, amount);

        submitValidatorSignatures(messageId);

        vm.expectRevert(abi.encodeWithSelector(Bridge.InsufficientWrappedNativeTokenBalance.selector, amount, 0));
        bridge.handleMessage(messageId);
    }

    function test_HandleMessage_PartialWETH_Reverts() public {
        bytes32 messageId = keccak256("partial-weth");
        bytes memory payload = "";
        uint256 required = 1 ether;
        uint256 available = 0.5 ether;
        SequencerSignature memory sig = createSequencerSignature(messageId, receiver, payload, required);

        // Deposit ETH to bridge first
        vm.prank(user);
        (bool success,) = address(bridge).call{value: required}("");
        assertTrue(success);

        vm.prank(sequencer);
        bridge.initializeMessage(messageId, receiver, payload, sig, required);

        vm.prank(address(bridge));
        weth.transfer(user, available);

        submitValidatorSignatures(messageId);

        vm.expectRevert(
            abi.encodeWithSelector(Bridge.InsufficientWrappedNativeTokenBalance.selector, required, available)
        );
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
        SequencerSignature memory sig = createSequencerSignature(messageId, receiver, payload, 0);

        uint256 bridgeWethBefore = weth.balanceOf(address(bridge));

        vm.prank(sequencer);
        bridge.initializeMessage(messageId, receiver, payload, sig, 0);

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
        uint256 amount = 1 ether;
        SequencerSignature memory sig = createSequencerSignature(messageId, address(reverter), payload, amount);

        vm.prank(sequencer);
        bridge.initializeMessage(messageId, address(reverter), payload, sig, amount);

        submitValidatorSignatures(messageId);

        vm.expectRevert();
        bridge.handleMessage(messageId);
    }

    function test_HandleMessage_SequencerOnlyMode_NoValidators() public {
        // Deploy a new set of contracts for sequencer-only mode
        MockAttestationVerifier newAttestationVerifier = new MockAttestationVerifier();
        TeeKeyManager newKeyManager = new TeeKeyManager(newAttestationVerifier);
        Bridge sequencerOnlyBridge = new Bridge(admin, address(weth), address(newKeyManager));
        newKeyManager.setBridge(address(sequencerOnlyBridge));

        // Register sequencer key and grant permission
        bytes memory publicValues = abi.encode(sequencer);
        sequencerOnlyBridge.registerKey(KeyType.Sequencer, publicValues, "");
        sequencerOnlyBridge.setMessageInitializer(sequencer, true);

        bytes32 messageId = keccak256("sequencer-only");
        bytes memory payload = "";
        SequencerSignature memory sig = createSequencerSignature(messageId, receiver, payload, 0);

        vm.prank(sequencer);
        sequencerOnlyBridge.initializeMessage(messageId, receiver, payload, sig, 0);

        // Should handle message successfully without any validator signatures
        sequencerOnlyBridge.handleMessage(messageId);

        assertTrue(sequencerOnlyBridge.isMessageCompleted(messageId));
        assertTrue(sequencerOnlyBridge.isMessageHandled(messageId));
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
                    WRAPNATIVETOKEN HELPER TESTS
    //////////////////////////////////////////////////////////////*/

    function test_WrapNativeToken_WrapsStuckETH() public {
        // Simulate stuck ETH by having bridge unwrap WETH
        vm.prank(user);
        weth.deposit{value: 10 ether}();
        vm.prank(user);
        weth.transfer(address(bridge), 10 ether);

        // Unwrap to create stuck ETH
        vm.prank(address(bridge));
        weth.withdraw(5 ether);

        uint256 stuckETH = address(bridge).balance;
        assertEq(stuckETH, 5 ether);

        uint256 bridgeWethBefore = weth.balanceOf(address(bridge));

        // Sequencer wraps the stuck ETH
        vm.prank(sequencer);
        bridge.wrapNativeToken(5 ether);

        // ETH should be wrapped
        assertEq(address(bridge).balance, 0);
        assertEq(weth.balanceOf(address(bridge)), bridgeWethBefore + 5 ether);
    }

    function test_WrapNativeToken_WrapsPartialAmount() public {
        // Create stuck ETH
        vm.prank(user);
        weth.deposit{value: 10 ether}();
        vm.prank(user);
        weth.transfer(address(bridge), 10 ether);
        vm.prank(address(bridge));
        weth.withdraw(5 ether);

        uint256 bridgeWethBefore = weth.balanceOf(address(bridge));

        // Wrap only 3 ETH of the 5 stuck
        vm.prank(sequencer);
        bridge.wrapNativeToken(3 ether);

        assertEq(address(bridge).balance, 2 ether);
        assertEq(weth.balanceOf(address(bridge)), bridgeWethBefore + 3 ether);
    }

    function test_WrapNativeToken_CapsAtBalance() public {
        // Create stuck ETH
        vm.prank(user);
        weth.deposit{value: 10 ether}();
        vm.prank(user);
        weth.transfer(address(bridge), 10 ether);
        vm.prank(address(bridge));
        weth.withdraw(5 ether);

        uint256 bridgeWethBefore = weth.balanceOf(address(bridge));

        // Try to wrap more than available
        vm.prank(sequencer);
        bridge.wrapNativeToken(100 ether);

        // Should only wrap what's available
        assertEq(address(bridge).balance, 0);
        assertEq(weth.balanceOf(address(bridge)), bridgeWethBefore + 5 ether);
    }

    function test_WrapNativeToken_RevertsIfNoETH() public {
        // No stuck ETH
        assertEq(address(bridge).balance, 0);

        vm.prank(sequencer);
        vm.expectRevert(Bridge.NoNativeTokenToWrap.selector);
        bridge.wrapNativeToken(1 ether);
    }

    function test_WrapNativeToken_OnlySequencer() public {
        // Create stuck ETH
        vm.prank(user);
        weth.deposit{value: 10 ether}();
        vm.prank(user);
        weth.transfer(address(bridge), 10 ether);
        vm.prank(address(bridge));
        weth.withdraw(5 ether);

        vm.prank(user);
        vm.expectRevert();
        bridge.wrapNativeToken(1 ether);
    }

    /*//////////////////////////////////////////////////////////////
                INITIALIZEANDHANDLEMESSAGE TESTS
    //////////////////////////////////////////////////////////////*/

    function test_InitializeAndHandleMessage_WithETH() public {
        bytes32 messageId = keccak256("init-handle-eth");
        bytes memory payload = "";
        uint256 amount = 1 ether;
        SequencerSignature memory sig = createSequencerSignature(messageId, receiver, payload, amount);

        // Deposit ETH to bridge first
        vm.prank(user);
        (bool success,) = address(bridge).call{value: amount}("");
        assertTrue(success);

        bytes[] memory validatorSigs = new bytes[](2);
        for (uint256 i = 0; i < 2; i++) {
            validatorSigs[i] = signMessage(messageId, validatorPrivateKeys[i]);
        }

        uint256 receiverBalanceBefore = receiver.balance;

        vm.prank(sequencer);
        bridge.initializeAndHandleMessage(messageId, receiver, payload, sig, validatorSigs, amount);

        assertEq(receiver.balance, receiverBalanceBefore + amount);
        assertTrue(bridge.isMessageCompleted(messageId));
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
            ethAmounts[i] = (i + 1) * 0.1 ether;
            totalEth += ethAmounts[i];
            sigs[i] = createSequencerSignature(messageIds[i], targetAddresses[i], payloads[i], ethAmounts[i]);
        }

        // Deposit ETH to bridge first
        vm.prank(user);
        (bool success,) = address(bridge).call{value: totalEth}("");
        assertTrue(success);

        uint256 bridgeWethBefore = weth.balanceOf(address(bridge));

        vm.prank(sequencer);
        bridge.batchInitializeMessage(messageIds, targetAddresses, payloads, sigs, ethAmounts);

        for (uint256 i = 0; i < batchSize; i++) {
            assertTrue(bridge.isMessageInitialized(messageIds[i]));
        }

        // WETH balance should remain the same (ETH was already wrapped via receive())
        assertEq(weth.balanceOf(address(bridge)), bridgeWethBefore);
    }

    function test_BatchInitializeMessage_ArrayLengthMismatch_Reverts() public {
        bytes32[] memory messageIds = new bytes32[](2);
        address[] memory targetAddresses = new address[](1); // Mismatched length
        bytes[] memory payloads = new bytes[](2);
        SequencerSignature[] memory sigs = new SequencerSignature[](2);
        uint256[] memory ethAmounts = new uint256[](2);

        vm.prank(sequencer);
        vm.expectRevert(Bridge.ArrayLengthMismatch.selector);
        bridge.batchInitializeMessage(messageIds, targetAddresses, payloads, sigs, ethAmounts);
    }

    function test_BatchHandleMessage() public {
        uint256 batchSize = 3;
        bytes32[] memory messageIds = new bytes32[](batchSize);

        for (uint256 i = 0; i < batchSize; i++) {
            messageIds[i] = keccak256(abi.encodePacked("batch-handle", i));
            bytes memory payload = "";
            SequencerSignature memory sig = createSequencerSignature(messageIds[i], receiver, payload, 0);

            vm.prank(sequencer);
            bridge.initializeMessage(messageIds[i], receiver, payload, sig, 0);

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

    function testFuzz_HandleMessage_CorrectlyUnwrapsAndSendsETH(uint256 ethAmount) public {
        ethAmount = bound(ethAmount, 1 wei, type(uint96).max);

        bytes32 messageId = keccak256(abi.encodePacked("fuzz-handle", ethAmount));
        bytes memory payload = "";
        SequencerSignature memory sig = createSequencerSignature(messageId, receiver, payload, ethAmount);

        // Deposit ETH to bridge first
        vm.prank(user);
        (bool depositSuccess,) = address(bridge).call{value: ethAmount}("");
        assertTrue(depositSuccess);

        vm.prank(sequencer);
        bridge.initializeMessage(messageId, receiver, payload, sig, ethAmount);

        submitValidatorSignatures(messageId);

        uint256 receiverBalanceBefore = receiver.balance;
        uint256 bridgeWethBefore = weth.balanceOf(address(bridge));

        bridge.handleMessage(messageId);

        assertEq(receiver.balance, receiverBalanceBefore + ethAmount);
        assertEq(weth.balanceOf(address(bridge)), bridgeWethBefore - ethAmount);
        assertTrue(bridge.isMessageCompleted(messageId));
    }

    function testFuzz_HandleMessage_InsufficientWrappedNativeToken_AlwaysReverts(
        uint256 requiredAmount,
        uint256 stolenAmount
    ) public {
        requiredAmount = bound(requiredAmount, 1 ether, type(uint96).max);
        stolenAmount = bound(stolenAmount, 1 wei, requiredAmount);
        uint256 remainingAmount = requiredAmount - stolenAmount;

        bytes32 messageId = keccak256(abi.encodePacked("fuzz-insufficient", requiredAmount, stolenAmount));
        bytes memory payload = "";
        SequencerSignature memory sig = createSequencerSignature(messageId, receiver, payload, requiredAmount);

        // Deposit ETH to bridge first
        vm.prank(user);
        (bool success,) = address(bridge).call{value: requiredAmount}("");
        assertTrue(success);

        vm.prank(sequencer);
        bridge.initializeMessage(messageId, receiver, payload, sig, requiredAmount);

        vm.prank(address(bridge));
        weth.transfer(user, stolenAmount);

        submitValidatorSignatures(messageId);

        vm.expectRevert(
            abi.encodeWithSelector(
                Bridge.InsufficientWrappedNativeTokenBalance.selector, requiredAmount, remainingAmount
            )
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
            ethAmounts[i] = bound(uint256(keccak256(abi.encodePacked(seed, i))), 0, 10 ether);
            totalEth += ethAmounts[i];
            sigs[i] = createSequencerSignature(messageIds[i], targetAddresses[i], payloads[i], ethAmounts[i]);
        }

        vm.assume(totalEth <= type(uint96).max);

        // Deposit ETH to bridge first
        if (totalEth > 0) {
            vm.prank(user);
            (bool success,) = address(bridge).call{value: totalEth}("");
            assertTrue(success);
        }

        uint256 bridgeWethBefore = weth.balanceOf(address(bridge));

        vm.prank(sequencer);
        bridge.batchInitializeMessage(messageIds, targetAddresses, payloads, sigs, ethAmounts);

        for (uint256 i = 0; i < arrayLength; i++) {
            assertTrue(bridge.isMessageInitialized(messageIds[i]));
        }

        // WETH balance should remain the same (ETH was already wrapped via receive())
        assertEq(weth.balanceOf(address(bridge)), bridgeWethBefore);
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
        SequencerSignature memory sig = createSequencerSignature(messageId, receiver, payload, ethAmount);

        vm.prank(sequencer);
        bridge.initializeMessage(messageId, receiver, payload, sig, ethAmount);

        uint256 bridgeWethAfterInit = weth.balanceOf(address(bridge));
        // WETH balance should remain the same (no wrapping during initializeMessage)
        assertEq(bridgeWethAfterInit, bridgeWethAfterWrap);

        // Handle message
        submitValidatorSignatures(messageId);

        uint256 receiverBalanceBefore = receiver.balance;
        bridge.handleMessage(messageId);

        assertEq(receiver.balance, receiverBalanceBefore + ethAmount);
        assertEq(weth.balanceOf(address(bridge)), bridgeWethAfterInit - ethAmount);
    }

    /// @notice Test that ETH returned from external call is automatically re-wrapped
    function test_HandleMessage_ReturnsETH_AutoReWraps() public {
        // Deploy a contract that returns some ETH
        ETHReturningContract returner = new ETHReturningContract();
        vm.deal(address(returner), 10 ether);

        uint256 sentAmount = 5 ether;
        uint256 returnedAmount = 2 ether;

        // Deposit ETH to bridge
        vm.prank(user);
        (bool success,) = address(bridge).call{value: sentAmount}("");
        assertTrue(success);

        uint256 bridgeWethBefore = weth.balanceOf(address(bridge));

        // Initialize message to call returner
        bytes32 messageId = keccak256("eth-return-test");
        bytes memory payload = abi.encodeWithSelector(returner.acceptAndReturn.selector, returnedAmount);
        SequencerSignature memory sig = createSequencerSignature(messageId, address(returner), payload, sentAmount);

        vm.prank(sequencer);
        bridge.initializeMessage(messageId, address(returner), payload, sig, sentAmount);

        submitValidatorSignatures(messageId);

        // Handle message - bridge sends sentAmount ETH, returner returns returnedAmount ETH
        bridge.handleMessage(messageId);

        // Bridge should have: original WETH - sentAmount + returnedAmount (re-wrapped)
        uint256 expectedWeth = bridgeWethBefore - sentAmount + returnedAmount;
        assertEq(weth.balanceOf(address(bridge)), expectedWeth, "ETH should be auto re-wrapped via receive()");

        // Returner should have kept sentAmount - returnedAmount
        assertEq(address(returner).balance, 10 ether + sentAmount - returnedAmount);
    }

    receive() external payable {}
}

contract RevertingContract {
    function alwaysReverts() external payable {
        revert("Always reverts");
    }
}

contract ETHReturningContract {
    /// @notice Accepts ETH and returns a specified amount back to caller
    function acceptAndReturn(uint256 amountToReturn) external payable {
        require(msg.value >= amountToReturn, "Insufficient ETH sent");
        (bool success,) = msg.sender.call{value: amountToReturn}("");
        require(success, "ETH return failed");
    }
}
