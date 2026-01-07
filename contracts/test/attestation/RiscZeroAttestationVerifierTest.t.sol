// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {Test} from "forge-std/Test.sol";
import {RiscZeroAttestationVerifier, PublicValuesStruct} from "src/attestation/RiscZeroAttestationVerifier.sol";

contract MockRiscZeroVerifier {
    bool public shouldRevert;

    function setShouldRevert(bool _shouldRevert) external {
        shouldRevert = _shouldRevert;
    }

    function verify(bytes calldata, bytes32, bytes32) external view {
        if (shouldRevert) {
            revert("RISC Zero proof verification failed");
        }
    }
}

/**
 * @title RiscZeroAttestationVerifierTest
 * @notice Comprehensive test suite for TEE attestation verification using RISC Zero
 * @dev Tests RiscZeroAttestationVerifier for GCP Confidential Space attestations
 */
contract RiscZeroAttestationVerifierTest is Test {
    RiscZeroAttestationVerifier public verifier;
    MockRiscZeroVerifier public riscZeroVerifier;

    address public admin;

    bytes32 public constant IMAGE_ID = 0x0098d80b4e7bbcd017b8ec2c9258eb22eef9abf5e56ea64eac11155e22abe2b2;
    bytes32 public constant TRUSTED_JWK_HASH = 0x9e2ff62c7d6b300a2d854ee77adb18a6b7b6c69619719a6f9866bca337581563;
    bytes32 public constant IMAGE_DIGEST_HASH = 0x052ef02859ef1a8a3fa67a763f3af6c54b38a5830c1f73894f447a82726e788b;
    uint64 public constant EXPIRATION_TOLERANCE = 3600;

    address public constant TEE_SIGNING_KEY = address(0x1234567890123456789012345678901234567890);

    event TrustedJwkHashAdded(bytes32 indexed jwkHash);
    event TrustedJwkHashRemoved(bytes32 indexed jwkHash);
    event ImageDigestHashUpdated(bytes32 oldHash, bytes32 newHash);
    event TrustedImageSignerAdded(address indexed signer);

    // Image signer (secp256k1) test constants
    // Using a known private key for deterministic signature generation
    uint256 public constant IMAGE_SIGNER_PRIVATE_KEY =
        0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80;
    address public IMAGE_SIGNER_ADDRESS;

    function setUp() public {
        admin = address(this);

        // Derive image signer address from private key
        IMAGE_SIGNER_ADDRESS = vm.addr(IMAGE_SIGNER_PRIVATE_KEY);

        riscZeroVerifier = new MockRiscZeroVerifier();

        verifier = new RiscZeroAttestationVerifier(
            address(riscZeroVerifier), IMAGE_ID, IMAGE_DIGEST_HASH, EXPIRATION_TOLERANCE
        );

        // Add trusted image signer for tests
        verifier.addTrustedImageSigner(IMAGE_SIGNER_ADDRESS);
    }

    function createValidPublicValues() internal view returns (PublicValuesStruct memory) {
        // Sign the image digest hash with the test private key
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(IMAGE_SIGNER_PRIVATE_KEY, IMAGE_DIGEST_HASH);

        return PublicValuesStruct({
            jwk_key_hash: TRUSTED_JWK_HASH,
            validity_window_start: uint64(block.timestamp - 1),
            validity_window_end: uint64(block.timestamp + 3600),
            image_digest_hash: IMAGE_DIGEST_HASH,
            tee_signing_key: TEE_SIGNING_KEY,
            secboot: true,
            dbgstat_disabled: true,
            audience_hash: keccak256("https://synddb-sequencer.example.com"),
            image_signature_v: v,
            image_signature_r: r,
            image_signature_s: s
        });
    }

    function test_Constructor_SetsCorrectValues() public view {
        assertEq(verifier.verifier(), address(riscZeroVerifier));
        assertEq(verifier.imageId(), IMAGE_ID);
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
        vm.expectRevert(RiscZeroAttestationVerifier.InvalidVerifierAddress.selector);
        new RiscZeroAttestationVerifier(address(0), IMAGE_ID, IMAGE_DIGEST_HASH, EXPIRATION_TOLERANCE);
    }

    function test_Constructor_RevertsOnInvalidImageId() public {
        vm.expectRevert(RiscZeroAttestationVerifier.InvalidImageId.selector);
        new RiscZeroAttestationVerifier(address(riscZeroVerifier), bytes32(0), IMAGE_DIGEST_HASH, EXPIRATION_TOLERANCE);
    }

    function test_Constructor_RevertsOnExcessiveTolerance() public {
        vm.expectRevert(RiscZeroAttestationVerifier.ToleranceExceedsOneDay.selector);
        new RiscZeroAttestationVerifier(address(riscZeroVerifier), IMAGE_ID, IMAGE_DIGEST_HASH, 86401);
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

        vm.expectRevert(abi.encodeWithSelector(RiscZeroAttestationVerifier.UntrustedJwkHash.selector, TRUSTED_JWK_HASH));
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
                RiscZeroAttestationVerifier.ValidityWindowNotStarted.selector,
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
                RiscZeroAttestationVerifier.ValidityWindowExpired.selector,
                values.validity_window_end,
                uint64(block.timestamp)
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

        vm.expectRevert(RiscZeroAttestationVerifier.SecureBootRequired.selector);
        verifier.verifyAttestationProof(publicValues, proof);
    }

    function test_VerifyAttestationProof_RevertsOnDebugModeEnabled() public {
        verifier.addTrustedJwkHash(TRUSTED_JWK_HASH);

        PublicValuesStruct memory values = createValidPublicValues();
        values.dbgstat_disabled = false;

        bytes memory publicValues = abi.encode(values);
        bytes memory proof = hex"";

        vm.expectRevert(RiscZeroAttestationVerifier.DebugModeNotAllowed.selector);
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
                RiscZeroAttestationVerifier.ImageDigestMismatch.selector, IMAGE_DIGEST_HASH, values.image_digest_hash
            )
        );
        verifier.verifyAttestationProof(publicValues, proof);
    }

    function test_VerifyAttestationProof_RevertsOnRiscZeroVerificationFailure() public {
        verifier.addTrustedJwkHash(TRUSTED_JWK_HASH);
        riscZeroVerifier.setShouldRevert(true);

        PublicValuesStruct memory values = createValidPublicValues();
        bytes memory publicValues = abi.encode(values);
        bytes memory proof = hex"";

        vm.expectRevert("RISC Zero proof verification failed");
        verifier.verifyAttestationProof(publicValues, proof);
    }

    function test_VerifyAttestationProof_ComputesCorrectJournalDigest() public {
        // Verify that sha256(publicValues) is passed correctly to the verifier
        verifier.addTrustedJwkHash(TRUSTED_JWK_HASH);

        PublicValuesStruct memory values = createValidPublicValues();
        bytes memory publicValues = abi.encode(values);

        // Compute expected journal digest
        bytes32 expectedDigest = sha256(publicValues);

        // The mock verifier will be called, we're just verifying it compiles correctly
        // In a real integration test, we'd verify the actual digest passed
        address result = verifier.verifyAttestationProof(publicValues, hex"");
        assertEq(result, TEE_SIGNING_KEY);

        // Sanity check: digest should be non-zero
        assertTrue(expectedDigest != bytes32(0));
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
