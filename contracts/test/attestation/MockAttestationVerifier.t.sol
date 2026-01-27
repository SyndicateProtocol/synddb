// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {Test} from "forge-std/Test.sol";
import {MockAttestationVerifier} from "src/attestation/MockAttestationVerifier.sol";
import {TeeKeyManager} from "src/attestation/TeeKeyManager.sol";

contract MockAttestationVerifierTest is Test {
    MockAttestationVerifier public verifier;
    TeeKeyManager public keyManager;

    function setUp() public {
        verifier = new MockAttestationVerifier();
        keyManager = new TeeKeyManager(verifier);
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

    function test_TeeKeyManager_AddKeyWithMockVerifier() public {
        address signerAddress = makeAddr("sequencer-signer");

        // Encode the address as publicValues
        bytes memory publicValues = abi.encode(signerAddress);

        // Add the key - should succeed with mock verifier
        keyManager.addKey(publicValues, "");

        // Verify the key is now valid
        assertTrue(keyManager.isKeyValid(signerAddress));
    }

    function test_TeeKeyManager_AddKeyWithMockVerifier_EmitsEvent() public {
        address signerAddress = makeAddr("sequencer-signer");
        bytes memory publicValues = abi.encode(signerAddress);

        vm.expectEmit(true, false, false, false);
        emit TeeKeyManager.KeyAdded(signerAddress);

        keyManager.addKey(publicValues, "");
    }

    function test_TeeKeyManager_RejectsInvalidKey() public {
        address signerAddress = makeAddr("sequencer-signer");
        address otherAddress = makeAddr("other");

        // Add one key
        bytes memory publicValues = abi.encode(signerAddress);
        keyManager.addKey(publicValues, "");

        // Query for a different key should revert
        vm.expectRevert(abi.encodeWithSelector(TeeKeyManager.InvalidPublicKey.selector, otherAddress));
        keyManager.isKeyValid(otherAddress);
    }

    function test_TeeKeyManager_RejectsDuplicateKey() public {
        address signerAddress = makeAddr("sequencer-signer");
        bytes memory publicValues = abi.encode(signerAddress);

        // Add key first time
        keyManager.addKey(publicValues, "");

        // Try to add same key again - should revert
        vm.expectRevert(abi.encodeWithSelector(TeeKeyManager.KeyAlreadyExists.selector, signerAddress));
        keyManager.addKey(publicValues, "");
    }
}
