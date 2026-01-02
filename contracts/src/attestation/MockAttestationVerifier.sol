// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {IAttestationVerifier} from "src/interfaces/IAttestationVerifier.sol";

/**
 * @title MockAttestationVerifier
 * @notice Mock attestation verifier for local development and testing
 * @dev Bypasses all attestation checks and returns whatever address is passed in publicValues.
 *      This contract should ONLY be deployed on Anvil (chain ID 31337) or when explicitly
 *      enabled via DEPLOY_MOCK_ATTESTATION=true for testing on other networks.
 *
 *      SECURITY: This contract provides NO attestation guarantees. Never deploy to production.
 *
 * Usage:
 *   The caller encodes the desired TEE signing key address in the publicValues parameter.
 *   The proofBytes parameter is ignored entirely.
 *
 * Example:
 *   bytes memory publicValues = abi.encode(mySignerAddress);
 *   keyManager.addKey(publicValues, ""); // proofBytes can be empty
 */
contract MockAttestationVerifier is IAttestationVerifier {
    /**
     * @notice Returns the address encoded in publicValues without any verification
     * @dev For testing only - accepts any input as valid
     * @param publicValues ABI-encoded address of the TEE signing key
     * @param proofBytes Ignored - can be empty
     * @return The decoded address from publicValues
     */
    function verifyAttestationProof(bytes calldata publicValues, bytes calldata proofBytes)
        external
        pure
        returns (address)
    {
        // Silence unused variable warning
        proofBytes;

        // Simply decode and return the address - no verification
        return abi.decode(publicValues, (address));
    }
}
