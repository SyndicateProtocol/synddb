// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {Ownable} from "@openzeppelin/contracts/access/Ownable.sol";
import {EnumerableSet} from "@openzeppelin/contracts/utils/structs/EnumerableSet.sol";
import {ECDSA} from "@openzeppelin/contracts/utils/cryptography/ECDSA.sol";
import {MessageHashUtils} from "@openzeppelin/contracts/utils/cryptography/MessageHashUtils.sol";
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
    using ECDSA for bytes32;
    using MessageHashUtils for bytes32;

    event KeyAdded(address indexed key);
    event KeyRemoved(address indexed key);
    event KeysRevoked();

    error InvalidPublicKey(address publicKey);
    error KeyAlreadyExists(address publicKey);
    error FailedToRemoveKey(address publicKey);
    error InvalidSignature();
    error SignatureExpired();

    /// @notice EIP-712 domain separator
    bytes32 public immutable DOMAIN_SEPARATOR;

    /// @notice EIP-712 typehash for AddKey
    bytes32 public constant ADDKEY_TYPEHASH = keccak256("AddKey(bytes32 attestationHash,uint256 deadline)");

    IAttestationVerifier public attestationVerifier;

    EnumerableSet.AddressSet internal validKeys;

    /// @notice Nonces for replay protection (teeKey => nonce)
    mapping(address => uint256) public nonces;

    /**
     * @notice Constructs the TEE key manager
     * @param _attestationVerifier Address of the attestation verifier contract
     */
    constructor(IAttestationVerifier _attestationVerifier) Ownable(msg.sender) {
        attestationVerifier = _attestationVerifier;
        DOMAIN_SEPARATOR = keccak256(
            abi.encode(
                keccak256("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)"),
                keccak256("TeeKeyManager"),
                keccak256("1"),
                block.chainid,
                address(this)
            )
        );
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

    /**
     * @notice Registers a new TEE key via signature (for keys without gas)
     * @dev Allows a relayer to submit on behalf of a TEE key. The signature proves
     *      the key holder authorized this registration.
     * @param publicValues The encoded public values from the attestation
     * @param proofBytes The SP1 proof bytes
     * @param deadline Timestamp after which the signature expires
     * @param signature EIP-712 signature from the TEE key
     */
    function addKeyWithSignature(
        bytes calldata publicValues,
        bytes calldata proofBytes,
        uint256 deadline,
        bytes calldata signature
    ) external {
        if (block.timestamp > deadline) revert SignatureExpired();

        // Verify the attestation and get the key address
        address publicKey = attestationVerifier.verifyAttestationProof(publicValues, proofBytes);

        // Verify the signature is from the key being registered
        bytes32 attestationHash = keccak256(abi.encodePacked(publicValues, proofBytes));
        bytes32 structHash = keccak256(abi.encode(ADDKEY_TYPEHASH, attestationHash, deadline));
        bytes32 digest = keccak256(abi.encodePacked("\x19\x01", DOMAIN_SEPARATOR, structHash));

        address signer = digest.recover(signature);
        if (signer != publicKey) revert InvalidSignature();

        // Add the key
        bool added = validKeys.add(publicKey);
        if (!added) revert KeyAlreadyExists(publicKey);
        emit KeyAdded(publicKey);
    }

    /**
     * @notice Removes a single key from the valid set
     * @dev Only callable by contract owner
     * @param publicKey The key to remove
     */
    function removeKey(address publicKey) external onlyOwner {
        if (!validKeys.remove(publicKey)) revert InvalidPublicKey(publicKey);
        emit KeyRemoved(publicKey);
    }

    /**
     * @notice Gets all currently valid keys
     * @dev Useful for monitoring and off-chain validation
     * @return Array of valid key addresses
     */
    function getValidKeys() external view returns (address[] memory) {
        return validKeys.values();
    }

    /**
     * @notice Gets the count of valid keys
     * @return Number of currently valid keys
     */
    function keyCount() external view returns (uint256) {
        return validKeys.length();
    }
}
