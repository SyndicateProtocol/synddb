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
 * @param cosign_signature_r Cosign signature R component (P-256 / secp256r1)
 * @param cosign_signature_s Cosign signature S component (P-256 / secp256r1)
 * @param cosign_pubkey_x Cosign public key X coordinate (P-256 / secp256r1)
 * @param cosign_pubkey_y Cosign public key Y coordinate (P-256 / secp256r1)
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
    bytes32 cosign_signature_r;
    bytes32 cosign_signature_s;
    bytes32 cosign_pubkey_x;
    bytes32 cosign_pubkey_y;
}

// RIP-7212 P256 precompile address for secp256r1 signature verification
address constant P256_VERIFIER = 0x0000000000000000000000000000000000000100;

/**
 * @title AttestationVerifier
 * @notice Verifies TEE attestation proofs using SP1 zero-knowledge proofs
 * @dev Specifically designed for GCP Confidential Space attestations.
 *      Validates attestation tokens signed by Google's JWKS keys and verifies
 *      container image digests, secure boot status, validity windows, and
 *      cosign signatures over image digests using P-256 (secp256r1).
 */
contract AttestationVerifier is IAttestationVerifier, Ownable {
    address public immutable verifier;
    bytes32 public immutable attestationVerifierVKey;
    bytes32 public expectedImageDigestHash;
    uint64 public immutable expirationTolerance;

    mapping(bytes32 jwkHash => bool isTrusted) public trustedJwkHashes;

    /// @notice Mapping from cosign pubkey hash to whether it's trusted
    /// @dev Key is keccak256(abi.encodePacked(pubkey_x, pubkey_y))
    mapping(bytes32 cosignPubkeyHash => bool isTrusted) public trustedCosignPubkeys;

    event TrustedJwkHashAdded(bytes32 indexed jwkHash);
    event TrustedJwkHashRemoved(bytes32 indexed jwkHash);
    event ImageDigestHashUpdated(bytes32 oldHash, bytes32 newHash);
    event TrustedCosignPubkeyAdded(bytes32 indexed pubkeyHash);
    event TrustedCosignPubkeyRemoved(bytes32 indexed pubkeyHash);

    error InvalidVerifierAddress();
    error InvalidVerificationKey();
    error ToleranceExceedsOneDay();
    error UntrustedJwkHash(bytes32 jwkHash);
    error ValidityWindowNotStarted(uint64 start, uint64 current);
    error ValidityWindowExpired(uint64 end, uint64 current);
    error SecureBootRequired();
    error DebugModeNotAllowed();
    error ImageDigestMismatch(bytes32 expected, bytes32 actual);
    error UntrustedCosignPubkey(bytes32 pubkeyHash);
    error CosignSignatureInvalid();

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
     *      Also verifies the cosign signature over the image digest using the P256 precompile.
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

        // Verify the cosign pubkey is trusted
        bytes32 cosignPubkeyHash = keccak256(abi.encodePacked(values.cosign_pubkey_x, values.cosign_pubkey_y));
        if (!trustedCosignPubkeys[cosignPubkeyHash]) {
            revert UntrustedCosignPubkey(cosignPubkeyHash);
        }

        // Verify cosign signature over image_digest_hash using P256 precompile (RIP-7212)
        // The precompile expects: message_hash (32) || r (32) || s (32) || x (32) || y (32)
        bytes memory p256Input = abi.encodePacked(
            values.image_digest_hash,
            values.cosign_signature_r,
            values.cosign_signature_s,
            values.cosign_pubkey_x,
            values.cosign_pubkey_y
        );
        (bool success, bytes memory result) = P256_VERIFIER.staticcall(p256Input);
        if (!success || result.length == 0 || abi.decode(result, (uint256)) != 1) {
            revert CosignSignatureInvalid();
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
     * @notice Adds a trusted cosign public key
     * @dev Cosign pubkeys are used to verify container image signatures from CI.
     *      Only callable by contract owner to prevent unauthorized key additions.
     * @param pubkeyX The X coordinate of the P-256 public key
     * @param pubkeyY The Y coordinate of the P-256 public key
     */
    function addTrustedCosignPubkey(bytes32 pubkeyX, bytes32 pubkeyY) external onlyOwner {
        bytes32 pubkeyHash = keccak256(abi.encodePacked(pubkeyX, pubkeyY));
        trustedCosignPubkeys[pubkeyHash] = true;
        emit TrustedCosignPubkeyAdded(pubkeyHash);
    }

    /**
     * @notice Removes a cosign public key from the trusted set
     * @dev Only callable by contract owner to prevent unauthorized key removals.
     * @param pubkeyX The X coordinate of the P-256 public key
     * @param pubkeyY The Y coordinate of the P-256 public key
     */
    function removeTrustedCosignPubkey(bytes32 pubkeyX, bytes32 pubkeyY) external onlyOwner {
        bytes32 pubkeyHash = keccak256(abi.encodePacked(pubkeyX, pubkeyY));
        trustedCosignPubkeys[pubkeyHash] = false;
        emit TrustedCosignPubkeyRemoved(pubkeyHash);
    }

    /**
     * @notice Checks if a cosign public key is trusted
     * @param pubkeyX The X coordinate of the P-256 public key
     * @param pubkeyY The Y coordinate of the P-256 public key
     * @return True if the public key is trusted
     */
    function isCosignPubkeyTrusted(bytes32 pubkeyX, bytes32 pubkeyY) external view returns (bool) {
        bytes32 pubkeyHash = keccak256(abi.encodePacked(pubkeyX, pubkeyY));
        return trustedCosignPubkeys[pubkeyHash];
    }
}
