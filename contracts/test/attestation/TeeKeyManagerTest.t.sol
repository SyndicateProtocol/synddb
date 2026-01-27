// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {Test} from "forge-std/Test.sol";
import {TeeKeyManager} from "src/attestation/TeeKeyManager.sol";
import {Bridge} from "src/Bridge.sol";
import {IAttestationVerifier} from "src/interfaces/IAttestationVerifier.sol";
import {KeyType} from "src/types/DataTypes.sol";

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
 * @dev Tests removeKey, getKeys, keyCount, and addKeyWithSignature
 */
contract TeeKeyManagerTest is Test {
    TeeKeyManager public keyManager;
    Bridge public bridge;
    MockAttestationVerifierForKeyManager public mockVerifier;

    address public admin;
    address public weth;

    uint256 internal teePrivateKey;
    address internal teeAddress;

    event KeyAdded(KeyType indexed keyType, address indexed key, uint256 expiresAt);
    event KeyRemoved(KeyType indexed keyType, address indexed key);
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
        bridge.registerKey(KeyType.Sequencer, bytes("proof"), bytes("values"));

        assertEq(keyManager.keyCount(KeyType.Sequencer), 1);
        keyManager.isKeyValid(KeyType.Sequencer, teeAddress); // Should not revert

        // Remove the key
        vm.expectEmit(true, true, false, false);
        emit KeyRemoved(KeyType.Sequencer, teeAddress);

        bridge.removeKey(KeyType.Sequencer, teeAddress);

        assertEq(keyManager.keyCount(KeyType.Sequencer), 0);
        vm.expectRevert(abi.encodeWithSelector(TeeKeyManager.InvalidPublicKey.selector, teeAddress));
        keyManager.isKeyValid(KeyType.Sequencer, teeAddress);
    }

    function test_RemoveSequencerKey_RevertsWhenNotOwner() public {
        // Add a key first
        mockVerifier.setNextReturnedKey(teeAddress);
        bridge.registerKey(KeyType.Sequencer, bytes("proof"), bytes("values"));

        // Try to remove as non-owner
        vm.prank(address(0x123));
        vm.expectRevert();
        bridge.removeKey(KeyType.Sequencer, teeAddress);
    }

    function test_RemoveSequencerKey_RevertsWhenKeyNotExists() public {
        vm.expectRevert(abi.encodeWithSelector(TeeKeyManager.InvalidPublicKey.selector, teeAddress));
        bridge.removeKey(KeyType.Sequencer, teeAddress);
    }

    function test_GetSequencerKeys_Empty() public view {
        address[] memory keys = keyManager.getKeys(KeyType.Sequencer);
        assertEq(keys.length, 0);
    }

    function test_GetSequencerKeys_SingleKey() public {
        mockVerifier.setNextReturnedKey(teeAddress);
        bridge.registerKey(KeyType.Sequencer, bytes("proof"), bytes("values"));

        address[] memory keys = keyManager.getKeys(KeyType.Sequencer);
        assertEq(keys.length, 1);
        assertEq(keys[0], teeAddress);
    }

    function test_GetSequencerKeys_MultipleKeys() public {
        address key1 = address(0x1111);
        address key2 = address(0x2222);
        address key3 = address(0x3333);

        mockVerifier.setNextReturnedKey(key1);
        bridge.registerKey(KeyType.Sequencer, bytes("proof1"), bytes("values1"));

        mockVerifier.setNextReturnedKey(key2);
        bridge.registerKey(KeyType.Sequencer, bytes("proof2"), bytes("values2"));

        mockVerifier.setNextReturnedKey(key3);
        bridge.registerKey(KeyType.Sequencer, bytes("proof3"), bytes("values3"));

        address[] memory keys = keyManager.getKeys(KeyType.Sequencer);
        assertEq(keys.length, 3);
    }

    function test_KeyCount_TracksCorrectly() public {
        assertEq(keyManager.keyCount(KeyType.Sequencer), 0);

        // Add keys
        mockVerifier.setNextReturnedKey(address(0x1111));
        bridge.registerKey(KeyType.Sequencer, bytes("proof1"), bytes("values1"));
        assertEq(keyManager.keyCount(KeyType.Sequencer), 1);

        mockVerifier.setNextReturnedKey(address(0x2222));
        bridge.registerKey(KeyType.Sequencer, bytes("proof2"), bytes("values2"));
        assertEq(keyManager.keyCount(KeyType.Sequencer), 2);

        // Remove a key
        bridge.removeKey(KeyType.Sequencer, address(0x1111));
        assertEq(keyManager.keyCount(KeyType.Sequencer), 1);

        // Revoke all
        bridge.revokeAllKeys();
        assertEq(keyManager.keyCount(KeyType.Sequencer), 0);
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

        vm.expectEmit(true, true, false, false);
        emit KeyAdded(KeyType.Sequencer, teeAddress, 0);

        bridge.registerKeyWithSignature(KeyType.Sequencer, publicValues, proofBytes, deadline, signature);

        keyManager.isKeyValid(KeyType.Sequencer, teeAddress); // Should not revert
        assertEq(keyManager.keyCount(KeyType.Sequencer), 1);
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
        bridge.registerKeyWithSignature(KeyType.Sequencer, publicValues, proofBytes, deadline, signature);
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
        bridge.registerKeyWithSignature(KeyType.Sequencer, publicValues, proofBytes, deadline, signature);
    }

    function test_AddSequencerKeyWithSignature_RevertsWhenKeyAlreadyExists() public {
        bytes memory publicValues = bytes("publicValues");
        bytes memory proofBytes = bytes("proofBytes");
        uint256 deadline = block.timestamp + 3600;

        mockVerifier.setNextReturnedKey(teeAddress);

        // First, add the key directly
        bridge.registerKey(KeyType.Sequencer, bytes("other"), bytes("other"));

        // Create signature
        bytes32 attestationHash = keccak256(abi.encodePacked(publicValues, proofBytes));
        bytes32 structHash = keccak256(abi.encode(keyManager.ADDKEY_TYPEHASH(), attestationHash, deadline));
        bytes32 digest = keccak256(abi.encodePacked("\x19\x01", keyManager.DOMAIN_SEPARATOR(), structHash));

        (uint8 v, bytes32 r, bytes32 s) = vm.sign(teePrivateKey, digest);
        bytes memory signature = abi.encodePacked(r, s, v);

        vm.expectRevert(abi.encodeWithSelector(TeeKeyManager.KeyAlreadyExists.selector, teeAddress));
        bridge.registerKeyWithSignature(KeyType.Sequencer, publicValues, proofBytes, deadline, signature);
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

        bridge.registerKeyWithSignature(KeyType.Sequencer, publicValues, proofBytes, deadline, signature);

        keyManager.isKeyValid(KeyType.Sequencer, teeAddress); // Should not revert
    }

    function test_AddValidatorKey_Success() public {
        mockVerifier.setNextReturnedKey(teeAddress);
        bridge.registerKey(KeyType.Validator, bytes("proof"), bytes("values"));

        assertEq(keyManager.keyCount(KeyType.Validator), 1);
        keyManager.isKeyValid(KeyType.Validator, teeAddress); // Should not revert
    }

    function test_RemoveValidatorKey_Success() public {
        mockVerifier.setNextReturnedKey(teeAddress);
        bridge.registerKey(KeyType.Validator, bytes("proof"), bytes("values"));

        assertEq(keyManager.keyCount(KeyType.Validator), 1);

        bridge.removeKey(KeyType.Validator, teeAddress);

        assertEq(keyManager.keyCount(KeyType.Validator), 0);
        vm.expectRevert(abi.encodeWithSelector(TeeKeyManager.InvalidPublicKey.selector, teeAddress));
        keyManager.isKeyValid(KeyType.Validator, teeAddress);
    }

    function test_GetValidatorKeys_MultipleKeys() public {
        address key1 = address(0x1111);
        address key2 = address(0x2222);

        mockVerifier.setNextReturnedKey(key1);
        bridge.registerKey(KeyType.Validator, bytes("proof1"), bytes("values1"));

        mockVerifier.setNextReturnedKey(key2);
        bridge.registerKey(KeyType.Validator, bytes("proof2"), bytes("values2"));

        address[] memory keys = keyManager.getKeys(KeyType.Validator);
        assertEq(keys.length, 2);
    }
}
