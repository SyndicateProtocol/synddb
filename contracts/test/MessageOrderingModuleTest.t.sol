// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {Test} from "forge-std/Test.sol";
import {MessageOrderingModule} from "src/modules/MessageOrderingModule.sol";
import {ProcessingStage, SequencerSignature} from "src/types/DataTypes.sol";

/**
 * @title MessageOrderingModuleTest
 * @notice Comprehensive tests for the MessageOrderingModule
 * @dev Tests nonce validation, sequential ordering, and edge cases
 */
contract MessageOrderingModuleTest is Test {
    MessageOrderingModule public module;
    address public owner;
    address public user1;
    address public user2;

    bytes32 public identifier1;
    bytes32 public identifier2;

    event NonceValidated(bytes32 indexed identifier, uint256 nonce, uint256 nextExpectedNonce);
    event NonceReset(bytes32 indexed identifier, uint256 newNonce);

    function setUp() public {
        owner = address(this);
        user1 = makeAddr("user1");
        user2 = makeAddr("user2");

        module = new MessageOrderingModule(owner);

        // Create identifiers for testing
        identifier1 = keccak256(abi.encodePacked("chain1", user1));
        identifier2 = keccak256(abi.encodePacked("chain1", user2));
    }

    /*//////////////////////////////////////////////////////////////
                        BASIC NONCE VALIDATION
    //////////////////////////////////////////////////////////////*/

    function test_ValidateNonce_FirstNonce() public {
        // First nonce should be 0
        assertEq(module.getExpectedNonce(identifier1), 0);

        vm.expectEmit(true, false, false, true);
        emit NonceValidated(identifier1, 0, 1);

        assertTrue(module.validateNonce(identifier1, 0));
        assertEq(module.getExpectedNonce(identifier1), 1);
    }

    function test_ValidateNonce_Sequential() public {
        // Validate nonces 0, 1, 2 sequentially
        module.validateNonce(identifier1, 0);
        assertEq(module.getExpectedNonce(identifier1), 1);

        module.validateNonce(identifier1, 1);
        assertEq(module.getExpectedNonce(identifier1), 2);

        module.validateNonce(identifier1, 2);
        assertEq(module.getExpectedNonce(identifier1), 3);
    }

    function test_ValidateNonce_RevertsOnSkippedNonce() public {
        // Try to validate nonce 1 when expecting 0
        vm.expectRevert(abi.encodeWithSelector(MessageOrderingModule.InvalidNonce.selector, identifier1, 0, 1));
        module.validateNonce(identifier1, 1);
    }

    function test_ValidateNonce_RevertsOnReplayedNonce() public {
        // Validate nonce 0
        module.validateNonce(identifier1, 0);

        // Try to replay nonce 0
        vm.expectRevert(abi.encodeWithSelector(MessageOrderingModule.InvalidNonce.selector, identifier1, 1, 0));
        module.validateNonce(identifier1, 0);
    }

    function test_ValidateNonce_MultipleIdentifiers() public {
        // Different identifiers maintain independent nonce sequences
        module.validateNonce(identifier1, 0);
        module.validateNonce(identifier2, 0);

        assertEq(module.getExpectedNonce(identifier1), 1);
        assertEq(module.getExpectedNonce(identifier2), 1);

        module.validateNonce(identifier1, 1);
        module.validateNonce(identifier2, 1);

        assertEq(module.getExpectedNonce(identifier1), 2);
        assertEq(module.getExpectedNonce(identifier2), 2);
    }

    /*//////////////////////////////////////////////////////////////
                        CHECK FUNCTION TESTS
    //////////////////////////////////////////////////////////////*/

    function test_Check_OnlyValidatesPreExecution() public {
        bytes32 messageId = keccak256("message1");
        SequencerSignature memory sig = SequencerSignature({signature: new bytes(65), submittedAt: block.timestamp});

        // Should return true for all stages except PreExecution (with empty payload)
        assertTrue(module.check(messageId, ProcessingStage.NotStarted, "", sig));
        assertTrue(module.check(messageId, ProcessingStage.PreExecution, "", sig)); // Empty payload = no validation
        assertTrue(module.check(messageId, ProcessingStage.Executing, "", sig));
        assertTrue(module.check(messageId, ProcessingStage.PostExecution, "", sig));
        assertTrue(module.check(messageId, ProcessingStage.Completed, "", sig));
        assertTrue(module.check(messageId, ProcessingStage.Rejected, "", sig));
    }

    function test_Check_WithValidPayload() public {
        bytes32 messageId = keccak256("message1");
        SequencerSignature memory sig = SequencerSignature({signature: new bytes(65), submittedAt: block.timestamp});

        // Create payload with identifier and nonce
        bytes memory payload = abi.encodePacked(identifier1, uint256(0));

        // First call should succeed with nonce 0
        assertTrue(module.check(messageId, ProcessingStage.PreExecution, payload, sig));
        assertEq(module.getExpectedNonce(identifier1), 1);

        // Second call with nonce 1 should succeed
        payload = abi.encodePacked(identifier1, uint256(1));
        assertTrue(module.check(messageId, ProcessingStage.PreExecution, payload, sig));
        assertEq(module.getExpectedNonce(identifier1), 2);
    }

    function test_Check_WithInvalidNonce_Reverts() public {
        bytes32 messageId = keccak256("message1");
        SequencerSignature memory sig = SequencerSignature({signature: new bytes(65), submittedAt: block.timestamp});

        // Create payload with nonce 5 (but expecting 0)
        bytes memory payload = abi.encodePacked(identifier1, uint256(5));

        vm.expectRevert(abi.encodeWithSelector(MessageOrderingModule.InvalidNonce.selector, identifier1, 0, 5));
        module.check(messageId, ProcessingStage.PreExecution, payload, sig);
    }

    function test_Check_WithShortPayload_SkipsValidation() public {
        bytes32 messageId = keccak256("message1");
        SequencerSignature memory sig = SequencerSignature({signature: new bytes(65), submittedAt: block.timestamp});

        // Payload with only 32 bytes (less than required 64)
        bytes memory shortPayload = abi.encodePacked(identifier1);

        // Should skip validation and return true
        assertTrue(module.check(messageId, ProcessingStage.PreExecution, shortPayload, sig));

        // Expected nonce should still be 0 (no validation occurred)
        assertEq(module.getExpectedNonce(identifier1), 0);
    }

    function test_Check_WithPayloadContainingExtraData() public {
        bytes32 messageId = keccak256("message1");
        SequencerSignature memory sig = SequencerSignature({signature: new bytes(65), submittedAt: block.timestamp});

        // Payload with identifier, nonce, and extra data
        bytes memory payload = abi.encodePacked(identifier1, uint256(0), "extra data for actual function call");

        // Should extract first 64 bytes and validate
        assertTrue(module.check(messageId, ProcessingStage.PreExecution, payload, sig));
        assertEq(module.getExpectedNonce(identifier1), 1);
    }

    /*//////////////////////////////////////////////////////////////
                        NONCE RESET TESTS
    //////////////////////////////////////////////////////////////*/

    function test_ResetNonce_OnlyOwner() public {
        vm.prank(user1);
        vm.expectRevert();
        module.resetNonce(identifier1, 10);
    }

    function test_ResetNonce_Success() public {
        // Validate some nonces first
        module.validateNonce(identifier1, 0);
        module.validateNonce(identifier1, 1);
        assertEq(module.getExpectedNonce(identifier1), 2);

        // Reset to nonce 10
        vm.expectEmit(true, false, false, true);
        emit NonceReset(identifier1, 10);
        module.resetNonce(identifier1, 10);

        assertEq(module.getExpectedNonce(identifier1), 10);

        // Should now expect nonce 10
        module.validateNonce(identifier1, 10);
        assertEq(module.getExpectedNonce(identifier1), 11);
    }

    function test_BatchResetNonces_OnlyOwner() public {
        bytes32[] memory identifiers = new bytes32[](2);
        identifiers[0] = identifier1;
        identifiers[1] = identifier2;

        uint256[] memory nonces = new uint256[](2);
        nonces[0] = 5;
        nonces[1] = 10;

        vm.prank(user1);
        vm.expectRevert();
        module.batchResetNonces(identifiers, nonces);
    }

    function test_BatchResetNonces_Success() public {
        bytes32[] memory identifiers = new bytes32[](2);
        identifiers[0] = identifier1;
        identifiers[1] = identifier2;

        uint256[] memory nonces = new uint256[](2);
        nonces[0] = 5;
        nonces[1] = 10;

        module.batchResetNonces(identifiers, nonces);

        assertEq(module.getExpectedNonce(identifier1), 5);
        assertEq(module.getExpectedNonce(identifier2), 10);
    }

    function test_BatchResetNonces_ArrayLengthMismatch() public {
        bytes32[] memory identifiers = new bytes32[](2);
        identifiers[0] = identifier1;
        identifiers[1] = identifier2;

        uint256[] memory nonces = new uint256[](1);
        nonces[0] = 5;

        vm.expectRevert("Array length mismatch");
        module.batchResetNonces(identifiers, nonces);
    }

    /*//////////////////////////////////////////////////////////////
                        FUZZING TESTS
    //////////////////////////////////////////////////////////////*/

    function testFuzz_ValidateNonce_Sequential(uint8 count) public {
        vm.assume(count > 0 && count <= 100);

        for (uint256 i = 0; i < count; i++) {
            module.validateNonce(identifier1, i);
            assertEq(module.getExpectedNonce(identifier1), i + 1);
        }
    }

    function testFuzz_ResetNonce_AnyValue(uint256 newNonce) public {
        // Bound to prevent overflow when incrementing
        newNonce = bound(newNonce, 0, type(uint256).max - 1);

        module.resetNonce(identifier1, newNonce);
        assertEq(module.getExpectedNonce(identifier1), newNonce);

        // Should be able to validate from the new nonce
        module.validateNonce(identifier1, newNonce);
        assertEq(module.getExpectedNonce(identifier1), newNonce + 1);
    }

    function testFuzz_MultipleIdentifiers_IndependentNonces(bytes32 id1, bytes32 id2, uint8 count1, uint8 count2)
        public
    {
        vm.assume(id1 != id2);
        vm.assume(count1 > 0 && count1 <= 50);
        vm.assume(count2 > 0 && count2 <= 50);

        // Validate different number of nonces for each identifier
        for (uint256 i = 0; i < count1; i++) {
            module.validateNonce(id1, i);
        }

        for (uint256 i = 0; i < count2; i++) {
            module.validateNonce(id2, i);
        }

        assertEq(module.getExpectedNonce(id1), count1);
        assertEq(module.getExpectedNonce(id2), count2);
    }

    /*//////////////////////////////////////////////////////////////
                        INTEGRATION SCENARIOS
    //////////////////////////////////////////////////////////////*/

    function test_Scenario_CrossChainOrdering() public {
        // Simulate cross-chain message ordering
        uint256 sourceChainId = 1;
        address sender = user1;

        bytes32 identifier = keccak256(abi.encodePacked(sourceChainId, sender));

        // Process 5 sequential messages
        for (uint256 i = 0; i < 5; i++) {
            module.validateNonce(identifier, i);
        }

        assertEq(module.getExpectedNonce(identifier), 5);

        // Attempting to process out of order should fail
        vm.expectRevert(abi.encodeWithSelector(MessageOrderingModule.InvalidNonce.selector, identifier, 5, 3));
        module.validateNonce(identifier, 3);
    }

    function test_Scenario_EmergencyReset() public {
        // Simulate a scenario where nonces need emergency reset
        bytes32 identifier = keccak256(abi.encodePacked("emergency-account"));

        // Process some messages
        for (uint256 i = 0; i < 10; i++) {
            module.validateNonce(identifier, i);
        }

        assertEq(module.getExpectedNonce(identifier), 10);

        // Emergency: Reset nonce back to 0
        module.resetNonce(identifier, 0);

        // Should be able to start from 0 again
        module.validateNonce(identifier, 0);
        assertEq(module.getExpectedNonce(identifier), 1);
    }

    function test_Scenario_MultipleChainsSameSender() public {
        // Same sender on different chains should have independent nonces
        address sender = user1;
        uint256 chain1 = 1;
        uint256 chain2 = 2;

        bytes32 id1 = keccak256(abi.encodePacked(chain1, sender));
        bytes32 id2 = keccak256(abi.encodePacked(chain2, sender));

        // Process different number of messages on each chain
        module.validateNonce(id1, 0);
        module.validateNonce(id1, 1);
        module.validateNonce(id1, 2);

        module.validateNonce(id2, 0);

        assertEq(module.getExpectedNonce(id1), 3);
        assertEq(module.getExpectedNonce(id2), 1);
    }
}
