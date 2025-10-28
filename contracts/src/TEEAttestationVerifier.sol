// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import "@openzeppelin/contracts/access/Ownable.sol";

/**
 * @title TEEAttestationVerifier
 * @notice Verifies TEE attestations from validators using SP1 proofs and Lit Protocol
 * @dev Implements dual attestation for enhanced security
 */
contract TEEAttestationVerifier is Ownable {
    // ============ Constants ============
    uint256 public constant ATTESTATION_VALIDITY_PERIOD = 7 days;
    uint256 public constant MIN_LIT_NODES = 2; // Minimum Lit nodes required for verification

    // ============ State Variables ============

    // SP1 verification contract
    address public immutable sp1Verifier;

    // Lit Protocol configuration
    struct LitConfig {
        string ipfsCid; // IPFS CID of the Lit Action
        uint256 chainId;
        address[] allowedNodes;
        uint256 minNodes;
    }

    LitConfig public litConfig;

    // Attestation records
    struct Attestation {
        bytes32 measurementHash; // Hash of TEE measurements
        bytes32 sp1ProofHash; // Hash of SP1 proof
        bytes32 litVerificationHash; // Hash of Lit verification
        bytes32 wasmVersionHash; // WASM version running in TEE
        uint256 timestamp;
        bool isValid;
        string platform; // e.g., "gcp-confidential-space"
        bool isRelayer; // Whether this is a relayer attestation
    }

    mapping(address => Attestation) public attestations;

    // Expected measurements for validation
    struct ExpectedMeasurement {
        bytes32 imageDigest; // Docker image hash
        bytes32 enclaveHash; // TEE enclave measurement
        string platform;
        bool isActive;
    }

    mapping(bytes32 => ExpectedMeasurement) public expectedMeasurements;
    bytes32[] public measurementIds;

    // ============ Events ============
    event AttestationSubmitted(
        address indexed validator,
        bytes32 measurementHash,
        bytes32 sp1ProofHash,
        string platform
    );

    event AttestationVerified(
        address indexed validator,
        bytes32 attestationHash,
        uint256 timestamp
    );

    event AttestationRevoked(
        address indexed validator,
        string reason
    );

    event ExpectedMeasurementAdded(
        bytes32 indexed measurementId,
        bytes32 imageDigest,
        string platform
    );

    event ExpectedMeasurementRemoved(
        bytes32 indexed measurementId,
        string reason
    );

    event LitConfigUpdated(
        string ipfsCid,
        uint256 minNodes
    );

    // ============ Errors ============
    error InvalidAttestation();
    error AttestationExpired();
    error InvalidSP1Proof();
    error InvalidLitVerification();
    error UnauthorizedPlatform();
    error MeasurementMismatch();

    // ============ Constructor ============
    constructor(
        address _sp1Verifier,
        string memory _litActionCid,
        address[] memory _litNodes,
        uint256 _minLitNodes
    ) Ownable(msg.sender) {
        require(_sp1Verifier != address(0), "Invalid SP1 verifier");
        require(bytes(_litActionCid).length > 0, "Invalid Lit CID");
        require(_litNodes.length >= MIN_LIT_NODES, "Too few Lit nodes");
        require(_minLitNodes >= MIN_LIT_NODES, "Min nodes too low");

        sp1Verifier = _sp1Verifier;

        litConfig = LitConfig({
            ipfsCid: _litActionCid,
            chainId: block.chainid,
            allowedNodes: _litNodes,
            minNodes: _minLitNodes
        });
    }

    // ============ Main Functions ============

    /**
     * @notice Submit and verify a TEE attestation
     * @param validator The validator address
     * @param measurementData The TEE measurement data
     * @param sp1Proof The SP1 zero-knowledge proof
     * @param litVerification The Lit Protocol verification result
     * @param isRelayer Whether this is a relayer attestation
     */
    function submitAttestation(
        address validator,
        bytes calldata measurementData,
        bytes calldata sp1Proof,
        bytes calldata litVerification,
        bool isRelayer
    ) external returns (bytes32) {
        require(validator != address(0), "Invalid validator");

        // Parse measurement data
        (
            bytes32 imageDigest,
            bytes32 enclaveHash,
            string memory platform,
            uint256 timestamp,
            bytes32 wasmVersionHash
        ) = decodeMeasurementData(measurementData);

        // Check timestamp is recent
        require(
            block.timestamp - timestamp <= ATTESTATION_VALIDITY_PERIOD,
            "Measurement too old"
        );

        // Verify measurement against expected values
        bytes32 measurementId = keccak256(abi.encode(imageDigest, platform));
        ExpectedMeasurement memory expected = expectedMeasurements[measurementId];

        if (!expected.isActive) {
            revert UnauthorizedPlatform();
        }

        if (expected.imageDigest != imageDigest || expected.enclaveHash != enclaveHash) {
            revert MeasurementMismatch();
        }

        // Verify SP1 proof
        bytes32 sp1ProofHash = verifySP1Proof(
            validator,
            measurementData,
            sp1Proof
        );

        // Verify Lit Protocol verification
        bytes32 litVerificationHash = verifyLitProtocol(
            validator,
            measurementData,
            litVerification
        );

        // Create attestation record
        bytes32 measurementHash = keccak256(measurementData);

        attestations[validator] = Attestation({
            measurementHash: measurementHash,
            sp1ProofHash: sp1ProofHash,
            litVerificationHash: litVerificationHash,
            wasmVersionHash: wasmVersionHash,
            timestamp: block.timestamp,
            isValid: true,
            platform: platform,
            isRelayer: isRelayer
        });

        // Calculate overall attestation hash
        bytes32 attestationHash = keccak256(abi.encode(
            measurementHash,
            sp1ProofHash,
            litVerificationHash,
            block.timestamp
        ));

        emit AttestationSubmitted(validator, measurementHash, sp1ProofHash, platform);
        emit AttestationVerified(validator, attestationHash, block.timestamp);

        return attestationHash;
    }

    /**
     * @notice Verify an SP1 proof
     * @param validator The validator address
     * @param measurementData The measurement data
     * @param proof The SP1 proof
     */
    function verifySP1Proof(
        address validator,
        bytes calldata measurementData,
        bytes calldata proof
    ) internal returns (bytes32) {
        // Call SP1 verifier contract
        (bool success, bytes memory result) = sp1Verifier.call(
            abi.encodeWithSignature(
                "verifyProof(bytes,bytes,address)",
                proof,
                measurementData,
                validator
            )
        );

        if (!success) {
            revert InvalidSP1Proof();
        }

        // Decode verification result
        (bool isValid, bytes32 proofHash) = abi.decode(result, (bool, bytes32));

        if (!isValid) {
            revert InvalidSP1Proof();
        }

        return proofHash;
    }

    /**
     * @notice Verify Lit Protocol attestation
     * @param validator The validator address
     * @param measurementData The measurement data
     * @param litVerification The Lit verification data
     */
    function verifyLitProtocol(
        address validator,
        bytes calldata measurementData,
        bytes calldata litVerification
    ) internal view returns (bytes32) {
        // Decode Lit verification data
        (
            address[] memory signers,
            bytes[] memory signatures,
            bytes32 resultHash
        ) = decodeLitVerification(litVerification);

        // Verify we have enough Lit node signatures
        uint256 validSignatures = 0;

        for (uint256 i = 0; i < signers.length; i++) {
            // Check if signer is an allowed Lit node
            if (isAllowedLitNode(signers[i])) {
                // Verify signature
                bytes32 messageHash = keccak256(abi.encode(
                    validator,
                    measurementData,
                    resultHash,
                    litConfig.ipfsCid
                ));

                if (verifySignature(messageHash, signatures[i], signers[i])) {
                    validSignatures++;
                }
            }
        }

        if (validSignatures < litConfig.minNodes) {
            revert InvalidLitVerification();
        }

        return resultHash;
    }

    /**
     * @notice Check if an attestation is valid and current
     * @param validator The validator address
     */
    function isAttestationValid(address validator) external view returns (bool) {
        Attestation memory attestation = attestations[validator];

        if (!attestation.isValid) {
            return false;
        }

        // Check if attestation is still within validity period
        if (block.timestamp - attestation.timestamp > ATTESTATION_VALIDITY_PERIOD) {
            return false;
        }

        return true;
    }

    /**
     * @notice Get attestation details for a validator
     * @param validator The validator address
     */
    function getAttestation(address validator)
        external
        view
        returns (
            bytes32 measurementHash,
            bytes32 sp1ProofHash,
            bytes32 litVerificationHash,
            bytes32 wasmVersionHash,
            uint256 timestamp,
            bool isValid,
            string memory platform,
            bool isRelayer
        )
    {
        Attestation memory attestation = attestations[validator];
        return (
            attestation.measurementHash,
            attestation.sp1ProofHash,
            attestation.litVerificationHash,
            attestation.wasmVersionHash,
            attestation.timestamp,
            attestation.isValid && (block.timestamp - attestation.timestamp <= ATTESTATION_VALIDITY_PERIOD),
            attestation.platform,
            attestation.isRelayer
        );
    }

    // ============ Admin Functions ============

    /**
     * @notice Add an expected measurement configuration
     * @param imageDigest The Docker image digest
     * @param enclaveHash The TEE enclave measurement
     * @param platform The platform identifier
     */
    function addExpectedMeasurement(
        bytes32 imageDigest,
        bytes32 enclaveHash,
        string memory platform
    ) external onlyOwner {
        bytes32 measurementId = keccak256(abi.encode(imageDigest, platform));

        expectedMeasurements[measurementId] = ExpectedMeasurement({
            imageDigest: imageDigest,
            enclaveHash: enclaveHash,
            platform: platform,
            isActive: true
        });

        measurementIds.push(measurementId);

        emit ExpectedMeasurementAdded(measurementId, imageDigest, platform);
    }

    /**
     * @notice Remove an expected measurement
     * @param measurementId The measurement ID to remove
     * @param reason The reason for removal
     */
    function removeExpectedMeasurement(
        bytes32 measurementId,
        string memory reason
    ) external onlyOwner {
        expectedMeasurements[measurementId].isActive = false;
        emit ExpectedMeasurementRemoved(measurementId, reason);
    }

    /**
     * @notice Update Lit Protocol configuration
     * @param newIpfsCid The new IPFS CID for the Lit Action
     * @param newNodes The new list of allowed Lit nodes
     * @param newMinNodes The new minimum nodes requirement
     */
    function updateLitConfig(
        string memory newIpfsCid,
        address[] memory newNodes,
        uint256 newMinNodes
    ) external onlyOwner {
        require(bytes(newIpfsCid).length > 0, "Invalid CID");
        require(newNodes.length >= MIN_LIT_NODES, "Too few nodes");
        require(newMinNodes >= MIN_LIT_NODES, "Min nodes too low");

        litConfig.ipfsCid = newIpfsCid;
        litConfig.allowedNodes = newNodes;
        litConfig.minNodes = newMinNodes;

        emit LitConfigUpdated(newIpfsCid, newMinNodes);
    }

    /**
     * @notice Revoke an attestation
     * @param validator The validator whose attestation to revoke
     * @param reason The reason for revocation
     */
    function revokeAttestation(
        address validator,
        string memory reason
    ) external onlyOwner {
        attestations[validator].isValid = false;
        emit AttestationRevoked(validator, reason);
    }

    // ============ Helper Functions ============

    /**
     * @notice Decode measurement data
     */
    function decodeMeasurementData(bytes calldata data)
        internal
        pure
        returns (
            bytes32 imageDigest,
            bytes32 enclaveHash,
            string memory platform,
            uint256 timestamp,
            bytes32 wasmVersionHash
        )
    {
        return abi.decode(data, (bytes32, bytes32, string, uint256, bytes32));
    }

    /**
     * @notice Decode Lit verification data
     */
    function decodeLitVerification(bytes calldata data)
        internal
        pure
        returns (
            address[] memory signers,
            bytes[] memory signatures,
            bytes32 resultHash
        )
    {
        return abi.decode(data, (address[], bytes[], bytes32));
    }

    /**
     * @notice Check if an address is an allowed Lit node
     */
    function isAllowedLitNode(address node) internal view returns (bool) {
        for (uint256 i = 0; i < litConfig.allowedNodes.length; i++) {
            if (litConfig.allowedNodes[i] == node) {
                return true;
            }
        }
        return false;
    }

    /**
     * @notice Verify a signature
     */
    function verifySignature(
        bytes32 messageHash,
        bytes memory signature,
        address signer
    ) internal pure returns (bool) {
        bytes32 ethSignedHash = keccak256(
            abi.encodePacked("\x19Ethereum Signed Message:\n32", messageHash)
        );

        (uint8 v, bytes32 r, bytes32 s) = splitSignature(signature);
        address recovered = ecrecover(ethSignedHash, v, r, s);

        return recovered == signer;
    }

    /**
     * @notice Split a signature into v, r, s components
     */
    function splitSignature(bytes memory sig)
        internal
        pure
        returns (uint8 v, bytes32 r, bytes32 s)
    {
        require(sig.length == 65, "Invalid signature length");

        assembly {
            r := mload(add(sig, 32))
            s := mload(add(sig, 64))
            v := byte(0, mload(add(sig, 96)))
        }

        return (v, r, s);
    }

    // ============ View Functions ============

    /**
     * @notice Get all measurement IDs
     */
    function getMeasurementIds() external view returns (bytes32[] memory) {
        return measurementIds;
    }

    /**
     * @notice Get Lit node configuration
     */
    function getLitNodes() external view returns (address[] memory) {
        return litConfig.allowedNodes;
    }

    /**
     * @notice Check if a specific measurement is expected
     */
    function isMeasurementExpected(
        bytes32 imageDigest,
        string memory platform
    ) external view returns (bool) {
        bytes32 measurementId = keccak256(abi.encode(imageDigest, platform));
        return expectedMeasurements[measurementId].isActive;
    }
}