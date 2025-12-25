// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {ECDSA} from "@openzeppelin/contracts/utils/cryptography/ECDSA.sol";
import {MessageStage, MessageStateV2} from "src/types/DataTypes.sol";
import {InvalidSignature} from "src/types/Errors.sol";

/**
 * @title SignatureVerifier
 * @notice Library for EIP-712 typed data signature verification
 * @dev Implements the Message Passing Bridge signature scheme
 */
library SignatureVerifier {
    /// @notice EIP-712 typehash for the Message struct
    /// @dev keccak256("Message(bytes32 messageId,string messageType,bytes calldata_,bytes32 metadataHash,uint64 nonce,uint64 timestamp,bytes32 domain)")
    bytes32 public constant MESSAGE_TYPEHASH =
        keccak256("Message(bytes32 messageId,string messageType,bytes calldata_,bytes32 metadataHash,uint64 nonce,uint64 timestamp,bytes32 domain)");

    /// @notice EIP-712 domain typehash
    bytes32 public constant DOMAIN_TYPEHASH =
        keccak256("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)");

    /// @notice Bridge name for EIP-712 domain
    bytes32 public constant NAME_HASH = keccak256("SyndBridge");

    /// @notice Bridge version for EIP-712 domain
    bytes32 public constant VERSION_HASH = keccak256("1");

    /**
     * @notice Compute the EIP-712 domain separator
     * @param chainId The chain ID
     * @param verifyingContract The bridge contract address
     * @return The domain separator bytes32
     */
    function computeDomainSeparator(uint256 chainId, address verifyingContract) internal pure returns (bytes32) {
        return keccak256(abi.encode(DOMAIN_TYPEHASH, NAME_HASH, VERSION_HASH, chainId, verifyingContract));
    }

    /**
     * @notice Compute the EIP-712 struct hash for a message
     * @param messageId The message ID
     * @param state The message state containing all fields
     * @return The struct hash bytes32
     */
    function computeStructHash(bytes32 messageId, MessageStateV2 storage state) internal view returns (bytes32) {
        return keccak256(
            abi.encode(
                MESSAGE_TYPEHASH,
                messageId,
                keccak256(bytes(state.messageType)),
                keccak256(state.calldata_),
                state.metadataHash,
                state.nonce,
                state.timestamp,
                state.domain
            )
        );
    }

    /**
     * @notice Compute the EIP-712 struct hash from individual parameters
     * @dev Used when message state is not yet stored
     */
    function computeStructHashFromParams(
        bytes32 messageId,
        string calldata messageType,
        bytes calldata calldata_,
        bytes32 metadataHash,
        uint64 nonce,
        uint64 timestamp,
        bytes32 domain
    ) internal pure returns (bytes32) {
        return keccak256(
            abi.encode(
                MESSAGE_TYPEHASH,
                messageId,
                keccak256(bytes(messageType)),
                keccak256(calldata_),
                metadataHash,
                nonce,
                timestamp,
                domain
            )
        );
    }

    /**
     * @notice Compute the full EIP-712 digest
     * @param domainSeparator The domain separator
     * @param structHash The struct hash
     * @return The final digest to sign
     */
    function computeDigest(bytes32 domainSeparator, bytes32 structHash) internal pure returns (bytes32) {
        return keccak256(abi.encodePacked("\x19\x01", domainSeparator, structHash));
    }

    /**
     * @notice Verify a signature and recover the signer
     * @param domainSeparator The EIP-712 domain separator
     * @param messageId The message ID
     * @param state The message state
     * @param signature The ECDSA signature
     * @return signer The recovered signer address
     */
    function verifySignature(
        bytes32 domainSeparator,
        bytes32 messageId,
        MessageStateV2 storage state,
        bytes calldata signature
    ) internal view returns (address signer) {
        bytes32 structHash = computeStructHash(messageId, state);
        bytes32 digest = computeDigest(domainSeparator, structHash);

        signer = ECDSA.recover(digest, signature);

        if (signer == address(0)) {
            revert InvalidSignature(messageId, signer);
        }
    }

    /**
     * @notice Verify a signature with explicit message parameters
     * @dev Used when verifying before message is stored
     */
    function verifySignatureFromParams(
        bytes32 domainSeparator,
        bytes32 messageId,
        string calldata messageType,
        bytes calldata calldata_,
        bytes32 metadataHash,
        uint64 nonce,
        uint64 timestamp,
        bytes32 domain,
        bytes calldata signature
    ) internal pure returns (address signer) {
        bytes32 structHash =
            computeStructHashFromParams(messageId, messageType, calldata_, metadataHash, nonce, timestamp, domain);
        bytes32 digest = computeDigest(domainSeparator, structHash);

        signer = ECDSA.recover(digest, signature);

        if (signer == address(0)) {
            revert InvalidSignature(messageId, signer);
        }
    }
}
