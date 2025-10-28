// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import "@openzeppelin/contracts/utils/cryptography/ECDSA.sol";

/**
 * @title SignatureUtils
 * @notice Utility library for signature verification and validation
 */
library SignatureUtils {
    using ECDSA for bytes32;

    /**
     * @notice Verify multiple signatures from a set of signers
     * @param messageHash The message hash to verify
     * @param signatures Array of signatures
     * @param validSigners Mapping of valid signers
     * @return validCount Number of valid unique signatures
     * @return signers Array of recovered signers
     */
    function verifyMultipleSignatures(
        bytes32 messageHash,
        bytes[] memory signatures,
        mapping(address => bool) storage validSigners
    ) internal view returns (uint256 validCount, address[] memory signers) {
        signers = new address[](signatures.length);
        validCount = 0;

        for (uint256 i = 0; i < signatures.length; i++) {
            address signer = recoverSigner(messageHash, signatures[i]);

            // Check if signer is valid
            if (validSigners[signer]) {
                // Check for duplicates
                bool isDuplicate = false;
                for (uint256 j = 0; j < i; j++) {
                    if (signers[j] == signer) {
                        isDuplicate = true;
                        break;
                    }
                }

                if (!isDuplicate) {
                    signers[validCount] = signer;
                    validCount++;
                }
            }
        }
    }

    /**
     * @notice Recover signer address from signature
     * @param messageHash The message hash
     * @param signature The signature bytes
     * @return The recovered signer address
     */
    function recoverSigner(
        bytes32 messageHash,
        bytes memory signature
    ) internal pure returns (address) {
        return messageHash.recover(signature);
    }

    /**
     * @notice Check if signatures meet threshold requirement
     * @param signatures Array of signatures
     * @param messageHash The message to verify
     * @param validSigners Mapping of valid signers
     * @param threshold Required number of signatures
     * @return Whether threshold is met
     */
    function meetsThreshold(
        bytes[] memory signatures,
        bytes32 messageHash,
        mapping(address => bool) storage validSigners,
        uint256 threshold
    ) internal view returns (bool) {
        (uint256 validCount, ) = verifyMultipleSignatures(
            messageHash,
            signatures,
            validSigners
        );
        return validCount >= threshold;
    }

    /**
     * @notice Create EIP-712 domain separator
     * @param name The domain name
     * @param version The domain version
     * @return The domain separator
     */
    function createDomainSeparator(
        string memory name,
        string memory version
    ) internal view returns (bytes32) {
        return keccak256(
            abi.encode(
                keccak256("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)"),
                keccak256(bytes(name)),
                keccak256(bytes(version)),
                block.chainid,
                address(this)
            )
        );
    }

    /**
     * @notice Hash typed data according to EIP-712
     * @param domainSeparator The domain separator
     * @param structHash The struct hash
     * @return The final hash
     */
    function hashTypedData(
        bytes32 domainSeparator,
        bytes32 structHash
    ) internal pure returns (bytes32) {
        return keccak256(
            abi.encodePacked("\x19\x01", domainSeparator, structHash)
        );
    }
}