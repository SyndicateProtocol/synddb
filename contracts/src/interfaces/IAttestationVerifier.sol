// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

/**
 * @title IAttestationVerifier
 * @notice Interface for verifying TEE attestation proofs
 * @dev Implementations verify attestation documents from various TEE providers (e.g., GCP Confidential Space)
 */
interface IAttestationVerifier {
    /**
     * @notice Verifies an attestation proof and returns the TEE signing key
     * @param publicValues The encoded public values from the attestation
     * @param proofBytes The ZK proof bytes
     * @return The address of the TEE signing key extracted from the attestation
     */
    function verifyAttestationProof(bytes calldata publicValues, bytes calldata proofBytes)
        external
        view
        returns (address);
}
