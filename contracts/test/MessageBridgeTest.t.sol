// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {Test} from "forge-std/Test.sol";
import {MessageBridge} from "src/MessageBridge.sol";
import {MessageStage, MessageStateV2, ApplicationConfig, ValidatorInfo} from "src/types/DataTypes.sol";
import {
    NotPrimaryValidator,
    MessageTypeNotRegistered,
    ApplicationNotRegistered,
    InvalidNonce,
    MessageAlreadyInitialized,
    ValidatorNotAuthorized,
    MessageNotPending,
    ThresholdNotReached
} from "src/types/Errors.sol";

import {MockWETH} from "test/use-cases/mocks/MockWETH.sol";
import {MockTarget} from "test/mocks/MockTarget.sol";

contract MessageBridgeTest is Test {
    MessageBridge public bridge;
    MockWETH public weth;
    MockTarget public target;

    address public admin = makeAddr("admin");
    address public primaryValidator = makeAddr("primaryValidator");
    address public witnessValidator1 = makeAddr("witnessValidator1");
    address public witnessValidator2 = makeAddr("witnessValidator2");
    address public user = makeAddr("user");

    bytes32 public constant DOMAIN = keccak256("test-app");
    string public constant MESSAGE_TYPE = "setValue(uint256)";

    uint256 public primaryValidatorPk = 0x1;
    uint256 public witnessValidator1Pk = 0x2;
    uint256 public witnessValidator2Pk = 0x3;

    function setUp() public {
        // Create addresses from private keys
        primaryValidator = vm.addr(primaryValidatorPk);
        witnessValidator1 = vm.addr(witnessValidator1Pk);
        witnessValidator2 = vm.addr(witnessValidator2Pk);

        // Deploy contracts
        weth = new MockWETH();
        target = new MockTarget();

        // Deploy bridge with threshold of 2
        vm.prank(admin);
        bridge = new MessageBridge(admin, address(weth), 2);

        // Setup: Register validators
        vm.startPrank(admin);

        // Add primary validator first
        bridge.addWitnessValidator(primaryValidator, "");

        // Add witness validators
        bridge.addWitnessValidator(witnessValidator1, "");
        bridge.addWitnessValidator(witnessValidator2, "");

        // Register message type
        bridge.registerMessageType(MESSAGE_TYPE, address(target), bytes32(0), "");

        // Register application with primary validator
        bridge.registerApplication(
            DOMAIN,
            ApplicationConfig({
                primaryValidator: primaryValidator,
                expirationSeconds: 86400,
                requireWitnessSignatures: true,
                active: true
            })
        );

        vm.stopPrank();
    }

    // ============================================================
    // INITIALIZATION TESTS
    // ============================================================

    function test_InitializeMessage_Success() public {
        bytes memory calldata_ = abi.encodeWithSignature("setValue(uint256)", 42);
        bytes32 metadataHash = keccak256("metadata");
        uint64 nonce = 1;
        uint64 timestamp = uint64(block.timestamp);

        bytes32 messageId = bridge.computeMessageId(MESSAGE_TYPE, calldata_, metadataHash, nonce, timestamp, DOMAIN);

        vm.prank(primaryValidator);
        bridge.initializeMessage(messageId, MESSAGE_TYPE, calldata_, metadataHash, "", nonce, timestamp, DOMAIN);

        // Verify state
        MessageStateV2 memory state = bridge.getMessageState(messageId);
        assertEq(uint8(state.stage), uint8(MessageStage.Pending));
        assertEq(state.primaryValidator, primaryValidator);
        assertEq(state.nonce, nonce);
        assertEq(state.domain, DOMAIN);
    }

    function test_InitializeMessage_RevertIfNotPrimaryValidator() public {
        bytes memory calldata_ = abi.encodeWithSignature("setValue(uint256)", 42);
        bytes32 metadataHash = keccak256("metadata");
        uint64 nonce = 1;
        uint64 timestamp = uint64(block.timestamp);

        bytes32 messageId = bridge.computeMessageId(MESSAGE_TYPE, calldata_, metadataHash, nonce, timestamp, DOMAIN);

        vm.prank(user);
        vm.expectRevert(abi.encodeWithSelector(NotPrimaryValidator.selector, DOMAIN, user));
        bridge.initializeMessage(messageId, MESSAGE_TYPE, calldata_, metadataHash, "", nonce, timestamp, DOMAIN);
    }

    function test_InitializeMessage_RevertIfMessageTypeNotRegistered() public {
        bytes memory calldata_ = abi.encodeWithSignature("unknownFunction()");
        bytes32 metadataHash = keccak256("metadata");
        uint64 nonce = 1;
        uint64 timestamp = uint64(block.timestamp);
        string memory unknownType = "unknownFunction()";

        bytes32 messageId = bridge.computeMessageId(unknownType, calldata_, metadataHash, nonce, timestamp, DOMAIN);

        vm.prank(primaryValidator);
        vm.expectRevert(abi.encodeWithSelector(MessageTypeNotRegistered.selector, unknownType));
        bridge.initializeMessage(messageId, unknownType, calldata_, metadataHash, "", nonce, timestamp, DOMAIN);
    }

    function test_InitializeMessage_RevertIfInvalidNonce() public {
        bytes memory calldata_ = abi.encodeWithSignature("setValue(uint256)", 42);
        bytes32 metadataHash = keccak256("metadata");
        uint64 wrongNonce = 5; // Should be 1
        uint64 timestamp = uint64(block.timestamp);

        bytes32 messageId =
            bridge.computeMessageId(MESSAGE_TYPE, calldata_, metadataHash, wrongNonce, timestamp, DOMAIN);

        vm.prank(primaryValidator);
        vm.expectRevert(abi.encodeWithSelector(InvalidNonce.selector, DOMAIN, 1, wrongNonce));
        bridge.initializeMessage(messageId, MESSAGE_TYPE, calldata_, metadataHash, "", wrongNonce, timestamp, DOMAIN);
    }

    // ============================================================
    // SIGNING TESTS
    // ============================================================

    function test_SignMessage_Success() public {
        // Initialize message
        bytes memory calldata_ = abi.encodeWithSignature("setValue(uint256)", 42);
        bytes32 metadataHash = keccak256("metadata");
        uint64 nonce = 1;
        uint64 timestamp = uint64(block.timestamp);

        bytes32 messageId = bridge.computeMessageId(MESSAGE_TYPE, calldata_, metadataHash, nonce, timestamp, DOMAIN);

        vm.prank(primaryValidator);
        bridge.initializeMessage(messageId, MESSAGE_TYPE, calldata_, metadataHash, "", nonce, timestamp, DOMAIN);

        // Sign message with primary validator
        bytes32 structHash = _computeStructHash(messageId, MESSAGE_TYPE, calldata_, metadataHash, nonce, timestamp, DOMAIN);
        bytes32 digest = _computeDigest(structHash);

        (uint8 v, bytes32 r, bytes32 s) = vm.sign(primaryValidatorPk, digest);
        bytes memory signature = abi.encodePacked(r, s, v);

        bridge.signMessage(messageId, signature);

        // Verify signature count
        assertEq(bridge.getSignatureCount(messageId), 1);
        assertTrue(bridge.hasValidatorSigned(messageId, primaryValidator));
    }

    function test_SignMessage_ThresholdReached() public {
        // Initialize message
        bytes memory calldata_ = abi.encodeWithSignature("setValue(uint256)", 42);
        bytes32 metadataHash = keccak256("metadata");
        uint64 nonce = 1;
        uint64 timestamp = uint64(block.timestamp);

        bytes32 messageId = bridge.computeMessageId(MESSAGE_TYPE, calldata_, metadataHash, nonce, timestamp, DOMAIN);

        vm.prank(primaryValidator);
        bridge.initializeMessage(messageId, MESSAGE_TYPE, calldata_, metadataHash, "", nonce, timestamp, DOMAIN);

        bytes32 structHash = _computeStructHash(messageId, MESSAGE_TYPE, calldata_, metadataHash, nonce, timestamp, DOMAIN);
        bytes32 digest = _computeDigest(structHash);

        // Sign with validator 1
        (uint8 v1, bytes32 r1, bytes32 s1) = vm.sign(primaryValidatorPk, digest);
        bridge.signMessage(messageId, abi.encodePacked(r1, s1, v1));

        // Sign with validator 2
        (uint8 v2, bytes32 r2, bytes32 s2) = vm.sign(witnessValidator1Pk, digest);
        bridge.signMessage(messageId, abi.encodePacked(r2, s2, v2));

        // Verify threshold reached - message should be Ready
        assertEq(uint8(bridge.getMessageStage(messageId)), uint8(MessageStage.Ready));
        assertEq(bridge.getSignatureCount(messageId), 2);
    }

    // ============================================================
    // EXECUTION TESTS
    // ============================================================

    function test_ExecuteMessage_Success() public {
        // Initialize and sign to threshold
        bytes memory calldata_ = abi.encodeWithSignature("setValue(uint256)", 42);
        bytes32 metadataHash = keccak256("metadata");
        uint64 nonce = 1;
        uint64 timestamp = uint64(block.timestamp);

        bytes32 messageId = bridge.computeMessageId(MESSAGE_TYPE, calldata_, metadataHash, nonce, timestamp, DOMAIN);

        vm.prank(primaryValidator);
        bridge.initializeMessage(messageId, MESSAGE_TYPE, calldata_, metadataHash, "", nonce, timestamp, DOMAIN);

        bytes32 structHash = _computeStructHash(messageId, MESSAGE_TYPE, calldata_, metadataHash, nonce, timestamp, DOMAIN);
        bytes32 digest = _computeDigest(structHash);

        // Sign with 2 validators to meet threshold
        (uint8 v1, bytes32 r1, bytes32 s1) = vm.sign(primaryValidatorPk, digest);
        bridge.signMessage(messageId, abi.encodePacked(r1, s1, v1));

        (uint8 v2, bytes32 r2, bytes32 s2) = vm.sign(witnessValidator1Pk, digest);
        bridge.signMessage(messageId, abi.encodePacked(r2, s2, v2));

        // Execute
        bridge.executeMessage(messageId);

        // Verify state
        assertEq(uint8(bridge.getMessageStage(messageId)), uint8(MessageStage.Completed));
        assertEq(target.value(), 42);
    }

    // ============================================================
    // QUERY TESTS
    // ============================================================

    function test_GetApplicationConfig() public view {
        ApplicationConfig memory config = bridge.getApplicationConfig(DOMAIN);
        assertEq(config.primaryValidator, primaryValidator);
        assertEq(config.expirationSeconds, 86400);
        assertTrue(config.active);
    }

    function test_GetValidatorInfo() public view {
        ValidatorInfo memory info = bridge.getValidatorInfo(primaryValidator);
        assertTrue(info.active);
    }

    function test_GetLastNonce() public view {
        uint64 nonce = bridge.getLastNonce(DOMAIN);
        assertEq(nonce, 0);
    }

    // ============================================================
    // HELPER FUNCTIONS
    // ============================================================

    function _computeStructHash(
        bytes32 messageId,
        string memory messageType,
        bytes memory calldata_,
        bytes32 metadataHash,
        uint64 nonce,
        uint64 timestamp,
        bytes32 domain
    ) internal pure returns (bytes32) {
        bytes32 MESSAGE_TYPEHASH = keccak256(
            "Message(bytes32 messageId,string messageType,bytes calldata_,bytes32 metadataHash,uint64 nonce,uint64 timestamp,bytes32 domain)"
        );
        return keccak256(
            abi.encode(
                MESSAGE_TYPEHASH,
                messageId,
                keccak256(bytes(messageType)),
                keccak256(calldata_),
                metadataHash,
                nonce,
                timestamp,
                domain
            )
        );
    }

    function _computeDigest(bytes32 structHash) internal view returns (bytes32) {
        return keccak256(abi.encodePacked("\x19\x01", bridge.DOMAIN_SEPARATOR(), structHash));
    }
}
