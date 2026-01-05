// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {Test} from "forge-std/Test.sol";
import {TeeKeyManager} from "src/attestation/TeeKeyManager.sol";
import {Bridge} from "src/Bridge.sol";
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
    Bridge public bridge;
    MockAttestationVerifierForKeyManager public mockVerifier;

    address public admin;
    address public weth;

    uint256 internal teePrivateKey;
    address internal teeAddress;

    event SequencerKeyAdded(address indexed key, uint256 expiresAt);
    event KeyRemoved(address indexed key);
    event KeysRevoked();

    function setUp() public {
        admin = address(this);
        weth = makeAddr("weth");

        mockVerifier = new MockAttestationVerifierForKeyManager();
        keyManager = new TeeKeyManager(mockVerifier);

        // Deploy bridge and connect to key manager
        bridge = new Bridge(admin, weth, address(keyManager));
        keyManager.setBridge(address(bridge));

        // Create a test TEE key
        teePrivateKey = 0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef;
        teeAddress = vm.addr(teePrivateKey);
    }

    function test_RemoveSequencerKey_Success() public {
        // Add a key first
        mockVerifier.setNextReturnedKey(teeAddress);
        bridge.registerSequencerKey(bytes("proof"), bytes("values"));

        assertEq(keyManager.sequencerKeyCount(), 1);
        keyManager.isSequencerKeyValid(teeAddress); // Should not revert

        // Remove the key
        vm.expectEmit(true, false, false, false);
        emit KeyRemoved(teeAddress);

        bridge.removeSequencerKey(teeAddress);

        assertEq(keyManager.sequencerKeyCount(), 0);
        vm.expectRevert(abi.encodeWithSelector(TeeKeyManager.InvalidPublicKey.selector, teeAddress));
        keyManager.isSequencerKeyValid(teeAddress);
    }

    function test_RemoveSequencerKey_RevertsWhenNotOwner() public {
        // Add a key first
        mockVerifier.setNextReturnedKey(teeAddress);
        bridge.registerSequencerKey(bytes("proof"), bytes("values"));

        // Try to remove as non-owner
        vm.prank(address(0x123));
        vm.expectRevert();
        bridge.removeSequencerKey(teeAddress);
    }

    function test_RemoveSequencerKey_RevertsWhenKeyNotExists() public {
        vm.expectRevert(abi.encodeWithSelector(TeeKeyManager.InvalidPublicKey.selector, teeAddress));
        bridge.removeSequencerKey(teeAddress);
    }

    function test_GetSequencerKeys_Empty() public view {
        address[] memory keys = keyManager.getSequencerKeys();
        assertEq(keys.length, 0);
    }

    function test_GetSequencerKeys_SingleKey() public {
        mockVerifier.setNextReturnedKey(teeAddress);
        bridge.registerSequencerKey(bytes("proof"), bytes("values"));

        address[] memory keys = keyManager.getSequencerKeys();
        assertEq(keys.length, 1);
        assertEq(keys[0], teeAddress);
    }

    function test_GetSequencerKeys_MultipleKeys() public {
        address key1 = address(0x1111);
        address key2 = address(0x2222);
        address key3 = address(0x3333);

        mockVerifier.setNextReturnedKey(key1);
        bridge.registerSequencerKey(bytes("proof1"), bytes("values1"));

        mockVerifier.setNextReturnedKey(key2);
        bridge.registerSequencerKey(bytes("proof2"), bytes("values2"));

        mockVerifier.setNextReturnedKey(key3);
        bridge.registerSequencerKey(bytes("proof3"), bytes("values3"));

        address[] memory keys = keyManager.getSequencerKeys();
        assertEq(keys.length, 3);
    }

    function test_KeyCount_TracksCorrectly() public {
        assertEq(keyManager.sequencerKeyCount(), 0);

        // Add keys
        mockVerifier.setNextReturnedKey(address(0x1111));
        bridge.registerSequencerKey(bytes("proof1"), bytes("values1"));
        assertEq(keyManager.sequencerKeyCount(), 1);

        mockVerifier.setNextReturnedKey(address(0x2222));
        bridge.registerSequencerKey(bytes("proof2"), bytes("values2"));
        assertEq(keyManager.sequencerKeyCount(), 2);

        // Remove a key
        bridge.removeSequencerKey(address(0x1111));
        assertEq(keyManager.sequencerKeyCount(), 1);

        // Revoke all
        bridge.revokeAllKeys();
        assertEq(keyManager.sequencerKeyCount(), 0);
    }

    function test_AddSequencerKeyWithSignature_Success() public {
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
        emit SequencerKeyAdded(teeAddress, 0);

        bridge.registerSequencerKeyWithSignature(publicValues, proofBytes, deadline, signature);

        keyManager.isSequencerKeyValid(teeAddress); // Should not revert
        assertEq(keyManager.sequencerKeyCount(), 1);
    }

    function test_AddSequencerKeyWithSignature_RevertsWhenExpired() public {
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
        bridge.registerSequencerKeyWithSignature(publicValues, proofBytes, deadline, signature);
    }

    function test_AddSequencerKeyWithSignature_RevertsWhenWrongSigner() public {
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
        bridge.registerSequencerKeyWithSignature(publicValues, proofBytes, deadline, signature);
    }

    function test_AddSequencerKeyWithSignature_RevertsWhenKeyAlreadyExists() public {
        bytes memory publicValues = bytes("publicValues");
        bytes memory proofBytes = bytes("proofBytes");
        uint256 deadline = block.timestamp + 3600;

        mockVerifier.setNextReturnedKey(teeAddress);

        // First, add the key directly
        bridge.registerSequencerKey(bytes("other"), bytes("other"));

        // Create signature
        bytes32 attestationHash = keccak256(abi.encodePacked(publicValues, proofBytes));
        bytes32 structHash = keccak256(abi.encode(keyManager.ADDKEY_TYPEHASH(), attestationHash, deadline));
        bytes32 digest = keccak256(abi.encodePacked("\x19\x01", keyManager.DOMAIN_SEPARATOR(), structHash));

        (uint8 v, bytes32 r, bytes32 s) = vm.sign(teePrivateKey, digest);
        bytes memory signature = abi.encodePacked(r, s, v);

        vm.expectRevert(abi.encodeWithSelector(TeeKeyManager.KeyAlreadyExists.selector, teeAddress));
        bridge.registerSequencerKeyWithSignature(publicValues, proofBytes, deadline, signature);
    }

    function test_AddSequencerKeyWithSignature_AllowsRelayerSubmission() public {
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

        bridge.registerSequencerKeyWithSignature(publicValues, proofBytes, deadline, signature);

        keyManager.isSequencerKeyValid(teeAddress); // Should not revert
    }

    function test_AddValidatorKey_Success() public {
        mockVerifier.setNextReturnedKey(teeAddress);
        bridge.registerValidatorKey(bytes("proof"), bytes("values"));

        assertEq(keyManager.validatorKeyCount(), 1);
        keyManager.isValidatorKeyValid(teeAddress); // Should not revert
    }

    function test_RemoveValidatorKey_Success() public {
        mockVerifier.setNextReturnedKey(teeAddress);
        bridge.registerValidatorKey(bytes("proof"), bytes("values"));

        assertEq(keyManager.validatorKeyCount(), 1);

        bridge.removeValidatorKey(teeAddress);

        assertEq(keyManager.validatorKeyCount(), 0);
        vm.expectRevert(abi.encodeWithSelector(TeeKeyManager.InvalidPublicKey.selector, teeAddress));
        keyManager.isValidatorKeyValid(teeAddress);
    }

    function test_GetValidatorKeys_MultipleKeys() public {
        address key1 = address(0x1111);
        address key2 = address(0x2222);

        mockVerifier.setNextReturnedKey(key1);
        bridge.registerValidatorKey(bytes("proof1"), bytes("values1"));

        mockVerifier.setNextReturnedKey(key2);
        bridge.registerValidatorKey(bytes("proof2"), bytes("values2"));

        address[] memory keys = keyManager.getValidatorKeys();
        assertEq(keys.length, 2);
    }
}
