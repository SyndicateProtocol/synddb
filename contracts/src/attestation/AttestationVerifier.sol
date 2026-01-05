// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {ISP1Verifier} from "@sp1-contracts/ISP1Verifier.sol";
import {IAttestationVerifier} from "src/interfaces/IAttestationVerifier.sol";
import {Ownable} from "@openzeppelin/contracts/access/Ownable.sol";

/**
 * @notice Public values structure for GCP Confidential Space attestations
 * @dev Must match the Rust definition in the SP1 program
 * @param jwk_key_hash Hash of the JWK key ID that signed the attestation token (keccak256 of kid)
 * @param validity_window_start Token issued at timestamp (iat claim)
 * @param validity_window_end Token expiration timestamp (exp claim)
 * @param image_digest_hash Hash of the container image digest (keccak256 of the sha256:... string)
 * @param tee_signing_key Address derived from TEE public key (currently unused for GCP CS)
 * @param secboot Whether secure boot was enabled in the TEE
 * @param dbgstat_disabled True if dbgstat claim is "disabled" (production mode)
 * @param audience_hash Hash of the audience string (keccak256 of audience claim)
 * @param image_signature_v Image signature V component (secp256k1 / Ethereum native)
 * @param image_signature_r Image signature R component (secp256k1 / Ethereum native)
 * @param image_signature_s Image signature S component (secp256k1 / Ethereum native)
 */
struct PublicValuesStruct {
    bytes32 jwk_key_hash;
    uint64 validity_window_start;
    uint64 validity_window_end;
    bytes32 image_digest_hash;
    address tee_signing_key;
    bool secboot;
    bool dbgstat_disabled;
    bytes32 audience_hash;
    uint8 image_signature_v;
    bytes32 image_signature_r;
    bytes32 image_signature_s;
}

/**
 * @title AttestationVerifier
 * @notice Verifies TEE attestation proofs using SP1 zero-knowledge proofs
 * @dev Specifically designed for GCP Confidential Space attestations.
 *      Validates attestation tokens signed by Google's JWKS keys and verifies
 *      container image digests, secure boot status, validity windows, and
 *      image signatures using secp256k1 (Ethereum native).
 */
contract AttestationVerifier is IAttestationVerifier, Ownable {
    address public immutable verifier;
    bytes32 public immutable attestationVerifierVKey;
    bytes32 public expectedImageDigestHash;
    uint64 public immutable expirationTolerance;

    mapping(bytes32 jwkHash => bool isTrusted) public trustedJwkHashes;

    /// @notice Mapping from image signer address to whether it's trusted
    /// @dev Image signers are Ethereum addresses that sign container image digests
    mapping(address imageSigner => bool isTrusted) public trustedImageSigners;

    event TrustedJwkHashAdded(bytes32 indexed jwkHash);
    event TrustedJwkHashRemoved(bytes32 indexed jwkHash);
    event ImageDigestHashUpdated(bytes32 oldHash, bytes32 newHash);
    event TrustedImageSignerAdded(address indexed signer);
    event TrustedImageSignerRemoved(address indexed signer);

    error InvalidVerifierAddress();
    error InvalidVerificationKey();
    error ToleranceExceedsOneDay();
    error UntrustedJwkHash(bytes32 jwkHash);
    error ValidityWindowNotStarted(uint64 start, uint64 current);
    error ValidityWindowExpired(uint64 end, uint64 current);
    error SecureBootRequired();
    error DebugModeNotAllowed();
    error ImageDigestMismatch(bytes32 expected, bytes32 actual);
    error UntrustedImageSigner(address signer);
    error ImageSignatureInvalid();

    /**
     * @notice Constructs the attestation verifier
     * @param _verifier Address of the SP1 verifier contract
     * @param _attestationVerifierVKey Verification key for the attestation verification program
     * @param _expectedImageDigestHash Expected container image digest hash
     * @param _expirationTolerance Grace period in seconds after token expiration (max 1 day recommended)
     */
    constructor(
        address _verifier,
        bytes32 _attestationVerifierVKey,
        bytes32 _expectedImageDigestHash,
        uint64 _expirationTolerance
    ) Ownable(msg.sender) {
        if (_verifier == address(0)) revert InvalidVerifierAddress();
        if (_attestationVerifierVKey == bytes32(0)) revert InvalidVerificationKey();
        if (_expirationTolerance > 86400) revert ToleranceExceedsOneDay();

        verifier = _verifier;
        attestationVerifierVKey = _attestationVerifierVKey;
        expectedImageDigestHash = _expectedImageDigestHash;
        expirationTolerance = _expirationTolerance;
    }

    /**
     * @notice Verifies an attestation proof and extracts the TEE signing key
     * @dev Validates all attestation claims before accepting the proof.
     *      Also verifies the image signature using ecrecover.
     *      For GCP Confidential Space, the tee_signing_key field is always address(0)
     *      because GCP CS JWT tokens do not contain an embedded signing key.
     *      This field exists for compatibility with other TEE providers (e.g., AWS Nitro).
     * @param publicValues ABI-encoded PublicValuesStruct
     * @param proofBytes SP1 proof bytes
     * @return The TEE signing key address from the attestation (address(0) for GCP CS)
     */
    function verifyAttestationProof(bytes calldata publicValues, bytes calldata proofBytes)
        public
        view
        returns (address)
    {
        PublicValuesStruct memory values = abi.decode(publicValues, (PublicValuesStruct));

        if (!trustedJwkHashes[values.jwk_key_hash]) {
            revert UntrustedJwkHash(values.jwk_key_hash);
        }

        if (block.timestamp < values.validity_window_start) {
            revert ValidityWindowNotStarted(values.validity_window_start, uint64(block.timestamp));
        }

        if (block.timestamp > values.validity_window_end + expirationTolerance) {
            revert ValidityWindowExpired(values.validity_window_end, uint64(block.timestamp));
        }

        if (!values.secboot) {
            revert SecureBootRequired();
        }

        if (!values.dbgstat_disabled) {
            revert DebugModeNotAllowed();
        }

        if (values.image_digest_hash != expectedImageDigestHash) {
            revert ImageDigestMismatch(expectedImageDigestHash, values.image_digest_hash);
        }

        // Recover signer from image signature using ecrecover
        // The signature is over the image_digest_hash
        address signer = ecrecover(
            values.image_digest_hash, values.image_signature_v, values.image_signature_r, values.image_signature_s
        );

        if (signer == address(0)) {
            revert ImageSignatureInvalid();
        }

        if (!trustedImageSigners[signer]) {
            revert UntrustedImageSigner(signer);
        }

        ISP1Verifier(verifier).verifyProof(attestationVerifierVKey, publicValues, proofBytes);

        return values.tee_signing_key;
    }

    /**
     * @notice Adds a trusted JWK hash to the allowlist
     * @dev JWK hashes correspond to Google's signing keys from their JWKS endpoint.
     *      Only callable by contract owner to prevent unauthorized key additions.
     * @param jwkHash The keccak256 hash of the JWK key ID (kid)
     */
    function addTrustedJwkHash(bytes32 jwkHash) external onlyOwner {
        trustedJwkHashes[jwkHash] = true;
        emit TrustedJwkHashAdded(jwkHash);
    }

    /**
     * @notice Removes a JWK hash from the trusted set
     * @dev Only callable by contract owner to prevent unauthorized key removals.
     * @param jwkHash The JWK hash to remove
     */
    function removeTrustedJwkHash(bytes32 jwkHash) external onlyOwner {
        trustedJwkHashes[jwkHash] = false;
        emit TrustedJwkHashRemoved(jwkHash);
    }

    /**
     * @notice Updates the expected container image digest hash
     * @dev Used when deploying new container versions.
     *      Only callable by contract owner to prevent unauthorized image digest changes.
     * @param newHash The new expected image digest hash
     */
    function updateImageDigestHash(bytes32 newHash) external onlyOwner {
        bytes32 oldHash = expectedImageDigestHash;
        expectedImageDigestHash = newHash;
        emit ImageDigestHashUpdated(oldHash, newHash);
    }

    /**
     * @notice Adds a trusted image signer address
     * @dev Image signers are Ethereum addresses that sign container image digests from CI.
     *      Only callable by contract owner to prevent unauthorized signer additions.
     * @param signer The Ethereum address of the trusted image signer
     */
    function addTrustedImageSigner(address signer) external onlyOwner {
        trustedImageSigners[signer] = true;
        emit TrustedImageSignerAdded(signer);
    }

    /**
     * @notice Removes an image signer from the trusted set
     * @dev Only callable by contract owner to prevent unauthorized signer removals.
     * @param signer The Ethereum address to remove
     */
    function removeTrustedImageSigner(address signer) external onlyOwner {
        trustedImageSigners[signer] = false;
        emit TrustedImageSignerRemoved(signer);
    }

    /**
     * @notice Checks if an image signer is trusted
     * @param signer The Ethereum address to check
     * @return True if the signer is trusted
     */
    function isImageSignerTrusted(address signer) external view returns (bool) {
        return trustedImageSigners[signer];
    }
}
