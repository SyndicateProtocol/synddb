// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {Ownable} from "@openzeppelin/contracts/access/Ownable.sol";
import {EnumerableSet} from "@openzeppelin/contracts/utils/structs/EnumerableSet.sol";
import {ITeeKeyManager} from "src/interfaces/ITeeKeyManager.sol";
import {IAttestationVerifier} from "src/interfaces/IAttestationVerifier.sol";

/**
 * @title TeeKeyManager
 * @notice Manages signing keys from trusted execution environments
 * @dev Allows registration of TEE keys with valid attestation proofs.
 *      Keys are validated through the attestation verifier before being added.
 *      Supports GCP Confidential Space and other TEE providers via the generic IAttestationVerifier interface.
 */
contract TeeKeyManager is ITeeKeyManager, Ownable {
    using EnumerableSet for EnumerableSet.AddressSet;

    event KeyAdded(address indexed key);
    event KeysRevoked();

    error InvalidPublicKey(address publicKey);
    error KeyAlreadyExists(address publicKey);
    error FailedToRemoveKey(address publicKey);

    IAttestationVerifier public attestationVerifier;

    EnumerableSet.AddressSet internal validKeys;

    /**
     * @notice Constructs the TEE key manager
     * @param _attestationVerifier Address of the attestation verifier contract
     */
    constructor(IAttestationVerifier _attestationVerifier) Ownable(msg.sender) {
        attestationVerifier = _attestationVerifier;
    }

    /**
     * @notice Checks if a public key belongs to a verified TEE
     * @dev Reverts with InvalidPublicKey if the key is not in the valid set
     * @param publicKey The address to check
     * @return True if the key is valid
     */
    function isKeyValid(address publicKey) external view override returns (bool) {
        if (!validKeys.contains(publicKey)) {
            revert InvalidPublicKey(publicKey);
        }
        return true;
    }

    /**
     * @notice Registers a new TEE key with an attestation proof
     * @dev Anyone can add a key by providing a valid attestation proof
     * @param publicValues The encoded public values from the attestation
     * @param proofBytes The SP1 proof bytes
     */
    function addKey(bytes calldata publicValues, bytes calldata proofBytes) external {
        address publicKey = attestationVerifier.verifyAttestationProof(publicValues, proofBytes);
        bool added = validKeys.add(publicKey);
        if (!added) revert KeyAlreadyExists(publicKey);
        emit KeyAdded(publicKey);
    }

    /**
     * @notice Revokes all registered keys
     * @dev Only callable by contract owner, useful when rotating attestation verifier.
     *      WARNING: May run out of gas if many keys are registered. This is acceptable
     *      since the expected number of TEE keys is small (typically 1-5 sequencers).
     *      If gas limits become an issue, consider deploying a new contract instead.
     */
    function revokeAllKeys() public onlyOwner {
        address[] memory keys = validKeys.values();
        for (uint256 i = 0; i < keys.length; i++) {
            if (!validKeys.remove(keys[i])) revert FailedToRemoveKey(keys[i]);
        }
        emit KeysRevoked();
    }

    /**
     * @notice Updates the attestation verifier and revokes all existing keys
     * @dev Keys must be re-registered with the new verifier
     * @param _attestationVerifier The new attestation verifier contract
     */
    function updateAttestationVerifier(IAttestationVerifier _attestationVerifier) external onlyOwner {
        revokeAllKeys();
        attestationVerifier = _attestationVerifier;
    }
}
