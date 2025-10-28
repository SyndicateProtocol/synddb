// SPDX-License-Identifier: MIT
pragma solidity ^0.8.30;

/**
 * @title ITEEAttestationVerifier
 * @notice Interface for TEE attestation verification
 */
interface ITEEAttestationVerifier {
    // ============ Structs ============

    struct Attestation {
        bytes32 measurementHash;
        bytes32 sp1ProofHash;
        bytes32 litVerificationHash;
        bytes32 wasmVersionHash;
        uint256 timestamp;
        bool isValid;
        string platform;
        bool isRelayer;
    }

    struct ExpectedMeasurement {
        bytes32 imageDigest;
        bytes32 enclaveHash;
        string platform;
        bool isActive;
    }

    struct LitConfig {
        string ipfsCid;
        uint256 chainId;
        address[] allowedNodes;
        uint256 minNodes;
    }

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

    // ============ Functions ============

    function submitAttestation(
        address validator,
        bytes calldata measurementData,
        bytes calldata sp1Proof,
        bytes calldata litVerification,
        bool isRelayer
    ) external returns (bytes32);

    function isAttestationValid(address validator) external view returns (bool);

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
        );

    function addExpectedMeasurement(
        bytes32 imageDigest,
        bytes32 enclaveHash,
        string memory platform
    ) external;

    function removeExpectedMeasurement(
        bytes32 measurementId,
        string memory reason
    ) external;

    function revokeAttestation(
        address validator,
        string memory reason
    ) external;

    function getMeasurementIds() external view returns (bytes32[] memory);
    function getLitNodes() external view returns (address[] memory);

    function isMeasurementExpected(
        bytes32 imageDigest,
        string memory platform
    ) external view returns (bool);
}