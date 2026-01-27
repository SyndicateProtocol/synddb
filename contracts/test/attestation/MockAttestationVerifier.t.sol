// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {Test} from "forge-std/Test.sol";
import {MockAttestationVerifier} from "src/attestation/MockAttestationVerifier.sol";
import {TeeKeyManager} from "src/attestation/TeeKeyManager.sol";
import {Bridge} from "src/Bridge.sol";

contract MockAttestationVerifierTest is Test {
    MockAttestationVerifier public verifier;
    TeeKeyManager public keyManager;
    Bridge public bridge;

    address public admin;
    address public weth;

    function setUp() public {
        admin = address(this);
        weth = makeAddr("weth");

        verifier = new MockAttestationVerifier();
        keyManager = new TeeKeyManager(verifier);

        // Deploy bridge and connect to key manager
        bridge = new Bridge(admin, weth, address(keyManager));
        keyManager.setBridge(address(bridge));
    }

    function test_verifyAttestationProof_ReturnsEncodedAddress() public view {
        address expected = address(0x1234567890AbcdEF1234567890aBcdef12345678);
        bytes memory publicValues = abi.encode(expected);

        address result = verifier.verifyAttestationProof(publicValues, "");

        assertEq(result, expected);
    }

    function test_verifyAttestationProof_AcceptsEmptyProof() public view {
        address expected = address(this);
        bytes memory publicValues = abi.encode(expected);

        address result = verifier.verifyAttestationProof(publicValues, "");

        assertEq(result, expected);
    }

    function testFuzz_verifyAttestationProof_AnyAddress(address anyAddress) public view {
        bytes memory publicValues = abi.encode(anyAddress);

        address result = verifier.verifyAttestationProof(publicValues, "");

        assertEq(result, anyAddress);
    }

    function test_TeeKeyManager_AddSequencerKeyWithMockVerifier() public {
        address signerAddress = makeAddr("sequencer-signer");

        // Encode the address as publicValues
        bytes memory publicValues = abi.encode(signerAddress);

        // Add the key through bridge - should succeed with mock verifier
        bridge.registerSequencerKey(publicValues, "");

        // Verify the key is now valid (should not revert)
        keyManager.isSequencerKeyValid(signerAddress);
    }

    function test_TeeKeyManager_AddSequencerKeyWithMockVerifier_EmitsEvent() public {
        address signerAddress = makeAddr("sequencer-signer");
        bytes memory publicValues = abi.encode(signerAddress);

        vm.expectEmit(true, false, false, false);
        emit TeeKeyManager.SequencerKeyAdded(signerAddress, 0);

        bridge.registerSequencerKey(publicValues, "");
    }

    function test_TeeKeyManager_RejectsInvalidKey() public {
        address signerAddress = makeAddr("sequencer-signer");
        address otherAddress = makeAddr("other");

        // Add one key
        bytes memory publicValues = abi.encode(signerAddress);
        bridge.registerSequencerKey(publicValues, "");

        // Query for a different key should revert
        vm.expectRevert(abi.encodeWithSelector(TeeKeyManager.InvalidPublicKey.selector, otherAddress));
        keyManager.isSequencerKeyValid(otherAddress);
    }

    function test_TeeKeyManager_RejectsDuplicateSequencerKey() public {
        address signerAddress = makeAddr("sequencer-signer");
        bytes memory publicValues = abi.encode(signerAddress);

        // Add key first time
        bridge.registerSequencerKey(publicValues, "");

        // Try to add same key again - should revert
        vm.expectRevert(abi.encodeWithSelector(TeeKeyManager.KeyAlreadyExists.selector, signerAddress));
        bridge.registerSequencerKey(publicValues, "");
    }

    function test_TeeKeyManager_AddValidatorKeyWithMockVerifier() public {
        address signerAddress = makeAddr("validator-signer");

        // Encode the address as publicValues
        bytes memory publicValues = abi.encode(signerAddress);

        // Add the key through bridge - should succeed with mock verifier
        bridge.registerValidatorKey(publicValues, "");

        // Verify the key is now valid (should not revert)
        keyManager.isValidatorKeyValid(signerAddress);
    }

    function test_TeeKeyManager_RejectsDuplicateValidatorKey() public {
        address signerAddress = makeAddr("validator-signer");
        bytes memory publicValues = abi.encode(signerAddress);

        // Add key first time
        bridge.registerValidatorKey(publicValues, "");

        // Try to add same key again - should revert
        vm.expectRevert(abi.encodeWithSelector(TeeKeyManager.KeyAlreadyExists.selector, signerAddress));
        bridge.registerValidatorKey(publicValues, "");
    }
}
