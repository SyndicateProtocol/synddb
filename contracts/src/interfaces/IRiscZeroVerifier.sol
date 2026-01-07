// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.30;

/**
 * @title IRiscZeroVerifier
 * @notice Interface for RISC Zero Groth16 proof verification
 * @dev Based on risc0-ethereum release-3.0
 */
interface IRiscZeroVerifier {
    /**
     * @notice Verifies a RISC Zero Groth16 proof
     * @dev Reverts if verification fails
     * @param seal The Groth16 proof bytes (seal)
     * @param imageId The RISC Zero program image ID
     * @param journalDigest The SHA256 hash of the journal (public values)
     */
    function verify(bytes calldata seal, bytes32 imageId, bytes32 journalDigest) external view;
}
