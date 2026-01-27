// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {Test} from "forge-std/Test.sol";
import {TeeKeyManager} from "src/attestation/TeeKeyManager.sol";
import {IAttestationVerifier} from "src/interfaces/IAttestationVerifier.sol";

contract MockAttestationVerifierForKeyManager is IAttestationVerifier {
    address public nextReturnedKey;

    function setNextReturnedKey(address key) external {
        nextReturnedKey = key;
    }

    function verifyAttestationProof(bytes calldata, bytes calldata) external view override returns (address) {
        return nextReturnedKey;
    }
}

/**
 * @title TeeKeyManagerTest
 * @notice Test suite for TeeKeyManager key rotation functions
 * @dev Tests removeKey, getValidKeys, keyCount, and addKeyWithSignature
 */
contract TeeKeyManagerTest is Test {
    TeeKeyManager public keyManager;
    MockAttestationVerifierForKeyManager public mockVerifier;

    uint256 internal teePrivateKey;
    address internal teeAddress;

    event KeyAdded(address indexed key);
    event KeyRemoved(address indexed key);
    event KeysRevoked();

    function setUp() public {
        mockVerifier = new MockAttestationVerifierForKeyManager();
        keyManager = new TeeKeyManager(mockVerifier);

        // Create a test TEE key
        teePrivateKey = 0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef;
        teeAddress = vm.addr(teePrivateKey);
    }

    function test_RemoveKey_Success() public {
        // Add a key first
        mockVerifier.setNextReturnedKey(teeAddress);
        keyManager.addKey(bytes("proof"), bytes("values"));

        assertEq(keyManager.keyCount(), 1);
        assertTrue(keyManager.isKeyValid(teeAddress));

        // Remove the key
        vm.expectEmit(true, false, false, false);
        emit KeyRemoved(teeAddress);

        keyManager.removeKey(teeAddress);

        assertEq(keyManager.keyCount(), 0);
        vm.expectRevert(abi.encodeWithSelector(TeeKeyManager.InvalidPublicKey.selector, teeAddress));
        keyManager.isKeyValid(teeAddress);
    }

    function test_RemoveKey_RevertsWhenNotOwner() public {
        // Add a key first
        mockVerifier.setNextReturnedKey(teeAddress);
        keyManager.addKey(bytes("proof"), bytes("values"));

        // Try to remove as non-owner
        vm.prank(address(0x123));
        vm.expectRevert();
        keyManager.removeKey(teeAddress);
    }

    function test_RemoveKey_RevertsWhenKeyNotExists() public {
        vm.expectRevert(abi.encodeWithSelector(TeeKeyManager.InvalidPublicKey.selector, teeAddress));
        keyManager.removeKey(teeAddress);
    }

    function test_GetValidKeys_Empty() public view {
        address[] memory keys = keyManager.getValidKeys();
        assertEq(keys.length, 0);
    }

    function test_GetValidKeys_SingleKey() public {
        mockVerifier.setNextReturnedKey(teeAddress);
        keyManager.addKey(bytes("proof"), bytes("values"));

        address[] memory keys = keyManager.getValidKeys();
        assertEq(keys.length, 1);
        assertEq(keys[0], teeAddress);
    }

    function test_GetValidKeys_MultipleKeys() public {
        address key1 = address(0x1111);
        address key2 = address(0x2222);
        address key3 = address(0x3333);

        mockVerifier.setNextReturnedKey(key1);
        keyManager.addKey(bytes("proof1"), bytes("values1"));

        mockVerifier.setNextReturnedKey(key2);
        keyManager.addKey(bytes("proof2"), bytes("values2"));

        mockVerifier.setNextReturnedKey(key3);
        keyManager.addKey(bytes("proof3"), bytes("values3"));

        address[] memory keys = keyManager.getValidKeys();
        assertEq(keys.length, 3);
    }

    function test_KeyCount_TracksCorrectly() public {
        assertEq(keyManager.keyCount(), 0);

        // Add keys
        mockVerifier.setNextReturnedKey(address(0x1111));
        keyManager.addKey(bytes("proof1"), bytes("values1"));
        assertEq(keyManager.keyCount(), 1);

        mockVerifier.setNextReturnedKey(address(0x2222));
        keyManager.addKey(bytes("proof2"), bytes("values2"));
        assertEq(keyManager.keyCount(), 2);

        // Remove a key
        keyManager.removeKey(address(0x1111));
        assertEq(keyManager.keyCount(), 1);

        // Revoke all
        keyManager.revokeAllKeys();
        assertEq(keyManager.keyCount(), 0);
    }

    function test_AddKeyWithSignature_Success() public {
        bytes memory publicValues = bytes("publicValues");
        bytes memory proofBytes = bytes("proofBytes");
        uint256 deadline = block.timestamp + 3600;

        mockVerifier.setNextReturnedKey(teeAddress);

        // Create EIP-712 signature
        bytes32 attestationHash = keccak256(abi.encodePacked(publicValues, proofBytes));
        bytes32 structHash = keccak256(abi.encode(keyManager.ADDKEY_TYPEHASH(), attestationHash, deadline));
        bytes32 digest = keccak256(abi.encodePacked("\x19\x01", keyManager.DOMAIN_SEPARATOR(), structHash));

        (uint8 v, bytes32 r, bytes32 s) = vm.sign(teePrivateKey, digest);
        bytes memory signature = abi.encodePacked(r, s, v);

        vm.expectEmit(true, false, false, false);
        emit KeyAdded(teeAddress);

        keyManager.addKeyWithSignature(publicValues, proofBytes, deadline, signature);

        assertTrue(keyManager.isKeyValid(teeAddress));
        assertEq(keyManager.keyCount(), 1);
    }

    function test_AddKeyWithSignature_RevertsWhenExpired() public {
        bytes memory publicValues = bytes("publicValues");
        bytes memory proofBytes = bytes("proofBytes");
        uint256 deadline = block.timestamp - 1; // Expired

        mockVerifier.setNextReturnedKey(teeAddress);

        // Create signature (even though it's expired)
        bytes32 attestationHash = keccak256(abi.encodePacked(publicValues, proofBytes));
        bytes32 structHash = keccak256(abi.encode(keyManager.ADDKEY_TYPEHASH(), attestationHash, deadline));
        bytes32 digest = keccak256(abi.encodePacked("\x19\x01", keyManager.DOMAIN_SEPARATOR(), structHash));

        (uint8 v, bytes32 r, bytes32 s) = vm.sign(teePrivateKey, digest);
        bytes memory signature = abi.encodePacked(r, s, v);

        vm.expectRevert(TeeKeyManager.SignatureExpired.selector);
        keyManager.addKeyWithSignature(publicValues, proofBytes, deadline, signature);
    }

    function test_AddKeyWithSignature_RevertsWhenWrongSigner() public {
        bytes memory publicValues = bytes("publicValues");
        bytes memory proofBytes = bytes("proofBytes");
        uint256 deadline = block.timestamp + 3600;

        // Verifier returns different address than the signer
        mockVerifier.setNextReturnedKey(address(0x9999));

        // Create signature from teePrivateKey (but verifier returns different address)
        bytes32 attestationHash = keccak256(abi.encodePacked(publicValues, proofBytes));
        bytes32 structHash = keccak256(abi.encode(keyManager.ADDKEY_TYPEHASH(), attestationHash, deadline));
        bytes32 digest = keccak256(abi.encodePacked("\x19\x01", keyManager.DOMAIN_SEPARATOR(), structHash));

        (uint8 v, bytes32 r, bytes32 s) = vm.sign(teePrivateKey, digest);
        bytes memory signature = abi.encodePacked(r, s, v);

        vm.expectRevert(TeeKeyManager.InvalidSignature.selector);
        keyManager.addKeyWithSignature(publicValues, proofBytes, deadline, signature);
    }

    function test_AddKeyWithSignature_RevertsWhenKeyAlreadyExists() public {
        bytes memory publicValues = bytes("publicValues");
        bytes memory proofBytes = bytes("proofBytes");
        uint256 deadline = block.timestamp + 3600;

        mockVerifier.setNextReturnedKey(teeAddress);

        // First, add the key directly
        keyManager.addKey(bytes("other"), bytes("other"));

        // Create signature
        bytes32 attestationHash = keccak256(abi.encodePacked(publicValues, proofBytes));
        bytes32 structHash = keccak256(abi.encode(keyManager.ADDKEY_TYPEHASH(), attestationHash, deadline));
        bytes32 digest = keccak256(abi.encodePacked("\x19\x01", keyManager.DOMAIN_SEPARATOR(), structHash));

        (uint8 v, bytes32 r, bytes32 s) = vm.sign(teePrivateKey, digest);
        bytes memory signature = abi.encodePacked(r, s, v);

        vm.expectRevert(abi.encodeWithSelector(TeeKeyManager.KeyAlreadyExists.selector, teeAddress));
        keyManager.addKeyWithSignature(publicValues, proofBytes, deadline, signature);
    }

    function test_AddKeyWithSignature_AllowsRelayerSubmission() public {
        bytes memory publicValues = bytes("publicValues");
        bytes memory proofBytes = bytes("proofBytes");
        uint256 deadline = block.timestamp + 3600;

        mockVerifier.setNextReturnedKey(teeAddress);

        // Create signature from TEE key
        bytes32 attestationHash = keccak256(abi.encodePacked(publicValues, proofBytes));
        bytes32 structHash = keccak256(abi.encode(keyManager.ADDKEY_TYPEHASH(), attestationHash, deadline));
        bytes32 digest = keccak256(abi.encodePacked("\x19\x01", keyManager.DOMAIN_SEPARATOR(), structHash));

        (uint8 v, bytes32 r, bytes32 s) = vm.sign(teePrivateKey, digest);
        bytes memory signature = abi.encodePacked(r, s, v);

        // Submit from a different address (relayer)
        address relayer = address(0xBEEF);
        vm.prank(relayer);

        keyManager.addKeyWithSignature(publicValues, proofBytes, deadline, signature);

        assertTrue(keyManager.isKeyValid(teeAddress));
    }
}
