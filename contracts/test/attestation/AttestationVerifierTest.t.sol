// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {Test} from "forge-std/Test.sol";
import {AttestationVerifier, PublicValuesStruct} from "src/attestation/AttestationVerifier.sol";
import {TeeKeyManager} from "src/attestation/TeeKeyManager.sol";
import {Bridge} from "src/Bridge.sol";

contract MockSP1Verifier {
    bool public shouldRevert;

    function setShouldRevert(bool _shouldRevert) external {
        shouldRevert = _shouldRevert;
    }

    function verifyProof(bytes32, bytes calldata, bytes calldata) external view {
        if (shouldRevert) {
            revert("SP1 proof verification failed");
        }
    }
}

/**
 * @title AttestationVerifierTest
 * @notice Comprehensive test suite for TEE attestation verification
 * @dev Tests AttestationVerifier and TeeKeyManager for GCP Confidential Space attestations
 */
contract AttestationVerifierTest is Test {
    AttestationVerifier public verifier;
    TeeKeyManager public keyManager;
    Bridge public bridge;
    MockSP1Verifier public sp1Verifier;

    address public admin;
    address public weth;

    bytes32 public constant VKEY = 0x0098d80b4e7bbcd017b8ec2c9258eb22eef9abf5e56ea64eac11155e22abe2b2;
    bytes32 public constant TRUSTED_JWK_HASH = 0x9e2ff62c7d6b300a2d854ee77adb18a6b7b6c69619719a6f9866bca337581563;
    bytes32 public constant IMAGE_DIGEST_HASH = 0x052ef02859ef1a8a3fa67a763f3af6c54b38a5830c1f73894f447a82726e788b;
    uint64 public constant EXPIRATION_TOLERANCE = 3600;

    address public constant TEE_SIGNING_KEY = address(0x1234567890123456789012345678901234567890);

    event TrustedJwkHashAdded(bytes32 indexed jwkHash);
    event TrustedJwkHashRemoved(bytes32 indexed jwkHash);
    event ImageDigestHashUpdated(bytes32 oldHash, bytes32 newHash);

    function setUp() public {
        admin = address(this);
        weth = makeAddr("weth");

        sp1Verifier = new MockSP1Verifier();

        verifier = new AttestationVerifier(address(sp1Verifier), VKEY, IMAGE_DIGEST_HASH, EXPIRATION_TOLERANCE);

        keyManager = new TeeKeyManager(verifier);

        // Deploy bridge and connect to key manager
        bridge = new Bridge(admin, weth, address(keyManager));
        keyManager.setBridge(address(bridge));
    }

    function createValidPublicValues() internal view returns (PublicValuesStruct memory) {
        return PublicValuesStruct({
            jwk_key_hash: TRUSTED_JWK_HASH,
            validity_window_start: uint64(block.timestamp - 1),
            validity_window_end: uint64(block.timestamp + 3600),
            image_digest_hash: IMAGE_DIGEST_HASH,
            tee_signing_key: TEE_SIGNING_KEY,
            secboot: true,
            dbgstat_disabled: true,
            audience_hash: keccak256("https://synddb-sequencer.example.com")
        });
    }

    function test_Constructor_SetsCorrectValues() public view {
        assertEq(verifier.verifier(), address(sp1Verifier));
        assertEq(verifier.attestationVerifierVKey(), VKEY);
        assertEq(verifier.expectedImageDigestHash(), IMAGE_DIGEST_HASH);
        assertEq(verifier.expirationTolerance(), EXPIRATION_TOLERANCE);
    }

    function test_AddTrustedJwkHash() public {
        vm.expectEmit(true, false, false, false);
        emit TrustedJwkHashAdded(TRUSTED_JWK_HASH);

        verifier.addTrustedJwkHash(TRUSTED_JWK_HASH);

        assertTrue(verifier.trustedJwkHashes(TRUSTED_JWK_HASH));
    }

    function test_AddTrustedJwkHash_RevertsWhenNotOwner() public {
        vm.prank(address(0x123));
        vm.expectRevert();
        verifier.addTrustedJwkHash(TRUSTED_JWK_HASH);
    }

    function test_RemoveTrustedJwkHash() public {
        verifier.addTrustedJwkHash(TRUSTED_JWK_HASH);

        vm.expectEmit(true, false, false, false);
        emit TrustedJwkHashRemoved(TRUSTED_JWK_HASH);

        verifier.removeTrustedJwkHash(TRUSTED_JWK_HASH);

        assertFalse(verifier.trustedJwkHashes(TRUSTED_JWK_HASH));
    }

    function test_RemoveTrustedJwkHash_RevertsWhenNotOwner() public {
        verifier.addTrustedJwkHash(TRUSTED_JWK_HASH);

        vm.prank(address(0x123));
        vm.expectRevert();
        verifier.removeTrustedJwkHash(TRUSTED_JWK_HASH);
    }

    function test_UpdateImageDigestHash() public {
        bytes32 newHash = keccak256("new_image_digest");

        vm.expectEmit(false, false, false, true);
        emit ImageDigestHashUpdated(IMAGE_DIGEST_HASH, newHash);

        verifier.updateImageDigestHash(newHash);

        assertEq(verifier.expectedImageDigestHash(), newHash);
    }

    function test_UpdateImageDigestHash_RevertsWhenNotOwner() public {
        bytes32 newHash = keccak256("new_image_digest");

        vm.prank(address(0x123));
        vm.expectRevert();
        verifier.updateImageDigestHash(newHash);
    }

    function test_Constructor_RevertsOnInvalidVerifier() public {
        vm.expectRevert(AttestationVerifier.InvalidVerifierAddress.selector);
        new AttestationVerifier(address(0), VKEY, IMAGE_DIGEST_HASH, EXPIRATION_TOLERANCE);
    }

    function test_Constructor_RevertsOnInvalidVKey() public {
        vm.expectRevert(AttestationVerifier.InvalidVerificationKey.selector);
        new AttestationVerifier(address(sp1Verifier), bytes32(0), IMAGE_DIGEST_HASH, EXPIRATION_TOLERANCE);
    }

    function test_Constructor_RevertsOnExcessiveTolerance() public {
        vm.expectRevert(AttestationVerifier.ToleranceExceedsOneDay.selector);
        new AttestationVerifier(address(sp1Verifier), VKEY, IMAGE_DIGEST_HASH, 86401);
    }

    function test_VerifyAttestationProof_Success() public {
        verifier.addTrustedJwkHash(TRUSTED_JWK_HASH);

        PublicValuesStruct memory values = createValidPublicValues();
        bytes memory publicValues = abi.encode(values);
        bytes memory proof = hex"";

        address result = verifier.verifyAttestationProof(publicValues, proof);

        assertEq(result, TEE_SIGNING_KEY);
    }

    function test_VerifyAttestationProof_RevertsOnUntrustedJwkHash() public {
        PublicValuesStruct memory values = createValidPublicValues();
        bytes memory publicValues = abi.encode(values);
        bytes memory proof = hex"";

        vm.expectRevert(abi.encodeWithSelector(AttestationVerifier.UntrustedJwkHash.selector, TRUSTED_JWK_HASH));
        verifier.verifyAttestationProof(publicValues, proof);
    }

    function test_VerifyAttestationProof_RevertsOnValidityWindowNotStarted() public {
        verifier.addTrustedJwkHash(TRUSTED_JWK_HASH);

        PublicValuesStruct memory values = createValidPublicValues();
        values.validity_window_start = uint64(block.timestamp + 1000);

        bytes memory publicValues = abi.encode(values);
        bytes memory proof = hex"";

        vm.expectRevert(
            abi.encodeWithSelector(
                AttestationVerifier.ValidityWindowNotStarted.selector,
                values.validity_window_start,
                uint64(block.timestamp)
            )
        );
        verifier.verifyAttestationProof(publicValues, proof);
    }

    function test_VerifyAttestationProof_RevertsOnValidityWindowExpired() public {
        // Set timestamp to a realistic value
        vm.warp(1700000000); // Nov 2023

        verifier.addTrustedJwkHash(TRUSTED_JWK_HASH);

        PublicValuesStruct memory values = createValidPublicValues();
        // Set end time in the past beyond tolerance
        values.validity_window_end = uint64(block.timestamp) - EXPIRATION_TOLERANCE - 1;

        bytes memory publicValues = abi.encode(values);
        bytes memory proof = hex"";

        vm.expectRevert(
            abi.encodeWithSelector(
                AttestationVerifier.ValidityWindowExpired.selector, values.validity_window_end, uint64(block.timestamp)
            )
        );
        verifier.verifyAttestationProof(publicValues, proof);
    }

    function test_VerifyAttestationProof_SucceedsWithinExpirationTolerance() public {
        // Set timestamp to a realistic value
        vm.warp(1700000000); // Nov 2023

        verifier.addTrustedJwkHash(TRUSTED_JWK_HASH);

        PublicValuesStruct memory values = createValidPublicValues();
        // Set end time in the past but within tolerance
        values.validity_window_end = uint64(block.timestamp) - EXPIRATION_TOLERANCE / 2;

        bytes memory publicValues = abi.encode(values);
        bytes memory proof = hex"";

        address result = verifier.verifyAttestationProof(publicValues, proof);
        assertEq(result, TEE_SIGNING_KEY);
    }

    function test_VerifyAttestationProof_RevertsOnSecureBootDisabled() public {
        verifier.addTrustedJwkHash(TRUSTED_JWK_HASH);

        PublicValuesStruct memory values = createValidPublicValues();
        values.secboot = false;

        bytes memory publicValues = abi.encode(values);
        bytes memory proof = hex"";

        vm.expectRevert(AttestationVerifier.SecureBootRequired.selector);
        verifier.verifyAttestationProof(publicValues, proof);
    }

    function test_VerifyAttestationProof_RevertsOnDebugModeEnabled() public {
        verifier.addTrustedJwkHash(TRUSTED_JWK_HASH);

        PublicValuesStruct memory values = createValidPublicValues();
        values.dbgstat_disabled = false;

        bytes memory publicValues = abi.encode(values);
        bytes memory proof = hex"";

        vm.expectRevert(AttestationVerifier.DebugModeNotAllowed.selector);
        verifier.verifyAttestationProof(publicValues, proof);
    }

    function test_VerifyAttestationProof_RevertsOnImageDigestMismatch() public {
        verifier.addTrustedJwkHash(TRUSTED_JWK_HASH);

        PublicValuesStruct memory values = createValidPublicValues();
        values.image_digest_hash = keccak256("wrong_image");

        bytes memory publicValues = abi.encode(values);
        bytes memory proof = hex"";

        vm.expectRevert(
            abi.encodeWithSelector(
                AttestationVerifier.ImageDigestMismatch.selector, IMAGE_DIGEST_HASH, values.image_digest_hash
            )
        );
        verifier.verifyAttestationProof(publicValues, proof);
    }

    function test_VerifyAttestationProof_RevertsOnSP1VerificationFailure() public {
        verifier.addTrustedJwkHash(TRUSTED_JWK_HASH);
        sp1Verifier.setShouldRevert(true);

        PublicValuesStruct memory values = createValidPublicValues();
        bytes memory publicValues = abi.encode(values);
        bytes memory proof = hex"";

        vm.expectRevert("SP1 proof verification failed");
        verifier.verifyAttestationProof(publicValues, proof);
    }

    function test_KeyManager_AddSequencerKey_Success() public {
        verifier.addTrustedJwkHash(TRUSTED_JWK_HASH);

        PublicValuesStruct memory values = createValidPublicValues();
        bytes memory publicValues = abi.encode(values);
        bytes memory proof = hex"";

        bridge.registerSequencerKey(publicValues, proof);

        keyManager.isSequencerKeyValid(TEE_SIGNING_KEY); // Should not revert
    }

    function test_KeyManager_AddSequencerKey_RevertsOnDuplicate() public {
        verifier.addTrustedJwkHash(TRUSTED_JWK_HASH);

        PublicValuesStruct memory values = createValidPublicValues();
        bytes memory publicValues = abi.encode(values);
        bytes memory proof = hex"";

        bridge.registerSequencerKey(publicValues, proof);

        vm.expectRevert(abi.encodeWithSelector(TeeKeyManager.KeyAlreadyExists.selector, TEE_SIGNING_KEY));
        bridge.registerSequencerKey(publicValues, proof);
    }

    function test_KeyManager_IsSequencerKeyValid_RevertsOnInvalidKey() public {
        vm.expectRevert(abi.encodeWithSelector(TeeKeyManager.InvalidPublicKey.selector, TEE_SIGNING_KEY));
        keyManager.isSequencerKeyValid(TEE_SIGNING_KEY);
    }

    function test_KeyManager_RevokeAllKeys() public {
        verifier.addTrustedJwkHash(TRUSTED_JWK_HASH);

        PublicValuesStruct memory values = createValidPublicValues();
        bytes memory publicValues = abi.encode(values);
        bytes memory proof = hex"";

        bridge.registerSequencerKey(publicValues, proof);
        keyManager.isSequencerKeyValid(TEE_SIGNING_KEY); // Should not revert

        bridge.revokeAllKeys();

        vm.expectRevert(abi.encodeWithSelector(TeeKeyManager.InvalidPublicKey.selector, TEE_SIGNING_KEY));
        keyManager.isSequencerKeyValid(TEE_SIGNING_KEY);
    }

    function test_KeyManager_UpdateAttestationVerifier() public {
        verifier.addTrustedJwkHash(TRUSTED_JWK_HASH);

        PublicValuesStruct memory values = createValidPublicValues();
        bytes memory publicValues = abi.encode(values);
        bytes memory proof = hex"";

        bridge.registerSequencerKey(publicValues, proof);
        keyManager.isSequencerKeyValid(TEE_SIGNING_KEY); // Should not revert

        AttestationVerifier newVerifier =
            new AttestationVerifier(address(sp1Verifier), VKEY, IMAGE_DIGEST_HASH, EXPIRATION_TOLERANCE);

        bridge.updateAttestationVerifier(newVerifier);

        // Keys are NOT automatically revoked when verifier is updated
        // Existing keys remain valid
        keyManager.isSequencerKeyValid(TEE_SIGNING_KEY); // Should still not revert

        assertEq(address(keyManager.attestationVerifier()), address(newVerifier));
    }

    function testFuzz_AddTrustedJwkHash(bytes32 jwkHash) public {
        verifier.addTrustedJwkHash(jwkHash);
        assertTrue(verifier.trustedJwkHashes(jwkHash));
    }

    function testFuzz_UpdateImageDigestHash(bytes32 newHash) public {
        verifier.updateImageDigestHash(newHash);
        assertEq(verifier.expectedImageDigestHash(), newHash);
    }

    function testFuzz_VerifyAttestationProof_WithVariousTimestamps(uint64 timestampOffset) public {
        vm.assume(timestampOffset > 0 && timestampOffset < 86400);

        // Set timestamp to a realistic value
        vm.warp(1700000000); // Nov 2023

        verifier.addTrustedJwkHash(TRUSTED_JWK_HASH);

        PublicValuesStruct memory values = createValidPublicValues();
        values.validity_window_start = uint64(block.timestamp) - timestampOffset;
        values.validity_window_end = uint64(block.timestamp) + timestampOffset;

        bytes memory publicValues = abi.encode(values);
        bytes memory proof = hex"";

        address result = verifier.verifyAttestationProof(publicValues, proof);
        assertEq(result, TEE_SIGNING_KEY);
    }
}
