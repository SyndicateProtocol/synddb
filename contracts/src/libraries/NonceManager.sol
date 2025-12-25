// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {InvalidNonce} from "src/types/Errors.sol";

/**
 * @title NonceManager
 * @notice Library for per-domain sequential nonce tracking
 * @dev Ensures messages are processed in order and prevents replay attacks
 */
library NonceManager {
    /**
     * @notice Validate and consume a nonce for a domain
     * @dev Reverts if nonce is not exactly lastNonce + 1
     * @param lastNonces Mapping of domain to last consumed nonce
     * @param domain The application domain
     * @param nonce The nonce to validate and consume
     */
    function validateAndConsume(mapping(bytes32 => uint64) storage lastNonces, bytes32 domain, uint64 nonce)
        internal
    {
        uint64 expected = lastNonces[domain] + 1;
        if (nonce != expected) {
            revert InvalidNonce(domain, expected, nonce);
        }
        lastNonces[domain] = nonce;
    }

    /**
     * @notice Get the next expected nonce for a domain
     * @param lastNonces Mapping of domain to last consumed nonce
     * @param domain The application domain
     * @return nextNonce The next valid nonce (lastNonce + 1)
     */
    function getNextNonce(mapping(bytes32 => uint64) storage lastNonces, bytes32 domain)
        internal
        view
        returns (uint64 nextNonce)
    {
        return lastNonces[domain] + 1;
    }

    /**
     * @notice Check if a nonce is valid (next expected nonce)
     * @param lastNonces Mapping of domain to last consumed nonce
     * @param domain The application domain
     * @param nonce The nonce to check
     * @return valid True if nonce is the next expected value
     */
    function isValidNonce(mapping(bytes32 => uint64) storage lastNonces, bytes32 domain, uint64 nonce)
        internal
        view
        returns (bool valid)
    {
        return nonce == lastNonces[domain] + 1;
    }

    /**
     * @notice Check if a nonce has been consumed
     * @param lastNonces Mapping of domain to last consumed nonce
     * @param domain The application domain
     * @param nonce The nonce to check
     * @return consumed True if nonce <= lastNonce
     */
    function isConsumed(mapping(bytes32 => uint64) storage lastNonces, bytes32 domain, uint64 nonce)
        internal
        view
        returns (bool consumed)
    {
        return nonce <= lastNonces[domain];
    }
}
