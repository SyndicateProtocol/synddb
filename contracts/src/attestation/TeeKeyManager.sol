// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {EnumerableSet} from "@openzeppelin/contracts/utils/structs/EnumerableSet.sol";
import {ECDSA} from "@openzeppelin/contracts/utils/cryptography/ECDSA.sol";
import {MessageHashUtils} from "@openzeppelin/contracts/utils/cryptography/MessageHashUtils.sol";
import {ITeeKeyManager} from "src/interfaces/ITeeKeyManager.sol";
import {IAttestationVerifier} from "src/interfaces/IAttestationVerifier.sol";
import {KeyType} from "src/types/DataTypes.sol";

/**
 * @title TeeKeyManager
 * @notice Manages signing keys from trusted execution environments for both sequencers and validators
 * @dev All state-changing operations are restricted to the Bridge contract.
 *      Keys are validated through the attestation verifier before being added.
 *      Supports optional expiration for registered keys.
 */
contract TeeKeyManager is ITeeKeyManager {
    using EnumerableSet for EnumerableSet.AddressSet;
    using ECDSA for bytes32;
    using MessageHashUtils for bytes32;

    /// @notice The Bridge contract that controls this key manager
    address public bridge;

    /// @notice EIP-712 domain separator
    bytes32 public immutable DOMAIN_SEPARATOR;

    /// @notice EIP-712 typehash for AddKey
    bytes32 public constant ADDKEY_TYPEHASH = keccak256("AddKey(bytes32 attestationHash,uint256 deadline)");

    IAttestationVerifier public attestationVerifier;

    EnumerableSet.AddressSet internal sequencerKeys;
    EnumerableSet.AddressSet internal validatorKeys;

    /// @notice Key expiration timestamps (0 = never expires)
    mapping(address key => uint256 expiresAt) public keyExpiration;

    /// @notice Pending keys awaiting owner approval
    mapping(address key => bool isPending) public pendingSequencerKeys;
    mapping(address key => bool isPending) public pendingValidatorKeys;

    /// @notice The deployer who can set the bridge address once
    address private immutable deployer;

    event BridgeSet(address indexed bridge);
    event KeyAdded(KeyType indexed keyType, address indexed key, uint256 expiresAt);
    event KeyPending(KeyType indexed keyType, address indexed key);
    event KeyRemoved(KeyType indexed keyType, address indexed key);
    event KeysRevoked();
    event KeyExpirationUpdated(address indexed key, uint256 expiresAt);
    event AttestationVerifierUpdated(address indexed oldVerifier, address indexed newVerifier);

    error InvalidPublicKey(address publicKey);
    error KeyAlreadyExists(address publicKey);
    error KeyExpired(address publicKey);
    error FailedToRemoveKey(address publicKey);
    error InvalidSignature();
    error SignatureExpired();
    error OnlyBridge();
    error OnlyDeployer();
    error BridgeAlreadySet();
    error KeyNotPending(address publicKey);
    error ZeroAddress();

    modifier onlyBridge() {
        if (msg.sender != bridge) revert OnlyBridge();
        _;
    }

    /**
     * @notice Constructs the TEE key manager
     * @param _attestationVerifier Address of the attestation verifier contract
     */
    constructor(IAttestationVerifier _attestationVerifier) {
        deployer = msg.sender;
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
     * @notice Sets the bridge address (can only be called once by deployer)
     * @dev This allows for deployment ordering: TeeKeyManager first, then Bridge
     * @param _bridge Address of the Bridge contract
     */
    function setBridge(address _bridge) external {
        if (msg.sender != deployer) revert OnlyDeployer();
        if (bridge != address(0)) revert BridgeAlreadySet();
        if (_bridge == address(0)) revert ZeroAddress();
        bridge = _bridge;
        emit BridgeSet(_bridge);
    }

    /*//////////////////////////////////////////////////////////////
                            INTERNAL HELPERS
    //////////////////////////////////////////////////////////////*/

    function _getKeySet(KeyType keyType) internal view returns (EnumerableSet.AddressSet storage) {
        return keyType == KeyType.Sequencer ? sequencerKeys : validatorKeys;
    }

    function _getPendingKeys(KeyType keyType, address key) internal view returns (bool) {
        return keyType == KeyType.Sequencer ? pendingSequencerKeys[key] : pendingValidatorKeys[key];
    }

    function _setPendingKey(KeyType keyType, address key, bool value) internal {
        if (keyType == KeyType.Sequencer) {
            pendingSequencerKeys[key] = value;
        } else {
            pendingValidatorKeys[key] = value;
        }
    }

    function _deletePendingKey(KeyType keyType, address key) internal {
        if (keyType == KeyType.Sequencer) {
            delete pendingSequencerKeys[key];
        } else {
            delete pendingValidatorKeys[key];
        }
    }

    /*//////////////////////////////////////////////////////////////
                            KEY VALIDATION
    //////////////////////////////////////////////////////////////*/

    /**
     * @notice Checks if a key is valid for the given type
     * @dev Reverts with InvalidPublicKey if key not registered, KeyExpired if expired
     * @param keyType The type of key (Sequencer or Validator)
     * @param publicKey The address to check
     * @return True if the key is valid
     */
    function isKeyValid(KeyType keyType, address publicKey) external view override returns (bool) {
        EnumerableSet.AddressSet storage keys = _getKeySet(keyType);
        if (!keys.contains(publicKey)) {
            revert InvalidPublicKey(publicKey);
        }
        uint256 expiry = keyExpiration[publicKey];
        if (expiry != 0 && block.timestamp > expiry) {
            revert KeyExpired(publicKey);
        }
        return true;
    }

    /*//////////////////////////////////////////////////////////////
                            KEY REGISTRATION
    //////////////////////////////////////////////////////////////*/

    /**
     * @notice Registers a key (called by Bridge)
     * @dev If registration is open, key is added directly. Otherwise, goes to pending.
     * @param keyType The type of key (Sequencer or Validator)
     * @param publicValues The encoded public values from the attestation
     * @param proofBytes The ZK proof bytes
     * @param requiresApproval Whether the key needs owner approval
     * @param expiresAt Expiration timestamp (0 = never expires)
     * @return publicKey The registered key address
     */
    function addKey(
        KeyType keyType,
        bytes calldata publicValues,
        bytes calldata proofBytes,
        bool requiresApproval,
        uint256 expiresAt
    ) external onlyBridge returns (address publicKey) {
        publicKey = attestationVerifier.verifyAttestationProof(publicValues, proofBytes);

        EnumerableSet.AddressSet storage keys = _getKeySet(keyType);
        if (keys.contains(publicKey)) revert KeyAlreadyExists(publicKey);

        if (requiresApproval) {
            _setPendingKey(keyType, publicKey, true);
            emit KeyPending(keyType, publicKey);
        } else {
            keys.add(publicKey);
            keyExpiration[publicKey] = expiresAt;
            emit KeyAdded(keyType, publicKey, expiresAt);
        }
    }

    /**
     * @notice Registers a key via signature (for keys without gas)
     * @param keyType The type of key (Sequencer or Validator)
     * @param publicValues The encoded public values from the attestation
     * @param proofBytes The ZK proof bytes
     * @param deadline Timestamp after which the signature expires
     * @param signature EIP-712 signature from the TEE key
     * @param requiresApproval Whether the key needs owner approval
     * @param expiresAt Expiration timestamp (0 = never expires)
     * @return publicKey The registered key address
     */
    function addKeyWithSignature(
        KeyType keyType,
        bytes calldata publicValues,
        bytes calldata proofBytes,
        uint256 deadline,
        bytes calldata signature,
        bool requiresApproval,
        uint256 expiresAt
    ) external onlyBridge returns (address publicKey) {
        if (block.timestamp > deadline) revert SignatureExpired();

        publicKey = attestationVerifier.verifyAttestationProof(publicValues, proofBytes);
        _verifyKeySignature(publicValues, proofBytes, deadline, signature, publicKey);

        EnumerableSet.AddressSet storage keys = _getKeySet(keyType);
        if (keys.contains(publicKey)) revert KeyAlreadyExists(publicKey);

        if (requiresApproval) {
            _setPendingKey(keyType, publicKey, true);
            emit KeyPending(keyType, publicKey);
        } else {
            keys.add(publicKey);
            keyExpiration[publicKey] = expiresAt;
            emit KeyAdded(keyType, publicKey, expiresAt);
        }
    }

    /**
     * @notice Verifies EIP-712 signature from TEE key
     */
    function _verifyKeySignature(
        bytes calldata publicValues,
        bytes calldata proofBytes,
        uint256 deadline,
        bytes calldata signature,
        address expectedSigner
    ) internal view {
        bytes32 attestationHash = keccak256(abi.encodePacked(publicValues, proofBytes));
        bytes32 structHash = keccak256(abi.encode(ADDKEY_TYPEHASH, attestationHash, deadline));
        bytes32 digest = keccak256(abi.encodePacked("\x19\x01", DOMAIN_SEPARATOR, structHash));

        address signer = digest.recover(signature);
        if (signer != expectedSigner) revert InvalidSignature();
    }

    /*//////////////////////////////////////////////////////////////
                            KEY APPROVAL
    //////////////////////////////////////////////////////////////*/

    /**
     * @notice Approves a pending key
     * @param keyType The type of key (Sequencer or Validator)
     * @param publicKey The pending key to approve
     * @param expiresAt Expiration timestamp (0 = never expires)
     */
    function approveKey(KeyType keyType, address publicKey, uint256 expiresAt) external onlyBridge {
        if (!_getPendingKeys(keyType, publicKey)) revert KeyNotPending(publicKey);

        _deletePendingKey(keyType, publicKey);
        EnumerableSet.AddressSet storage keys = _getKeySet(keyType);
        keys.add(publicKey);
        keyExpiration[publicKey] = expiresAt;
        emit KeyAdded(keyType, publicKey, expiresAt);
    }

    /**
     * @notice Rejects a pending key
     * @param keyType The type of key (Sequencer or Validator)
     * @param publicKey The pending key to reject
     */
    function rejectKey(KeyType keyType, address publicKey) external onlyBridge {
        if (!_getPendingKeys(keyType, publicKey)) revert KeyNotPending(publicKey);
        _deletePendingKey(keyType, publicKey);
    }

    /*//////////////////////////////////////////////////////////////
                            KEY MANAGEMENT
    //////////////////////////////////////////////////////////////*/

    /**
     * @notice Removes a key
     * @param keyType The type of key (Sequencer or Validator)
     * @param publicKey The key to remove
     */
    function removeKey(KeyType keyType, address publicKey) external onlyBridge {
        EnumerableSet.AddressSet storage keys = _getKeySet(keyType);
        if (!keys.remove(publicKey)) revert InvalidPublicKey(publicKey);
        delete keyExpiration[publicKey];
        emit KeyRemoved(keyType, publicKey);
    }

    /**
     * @notice Sets expiration for a key
     * @param publicKey The key to update
     * @param expiresAt New expiration timestamp (0 = never expires)
     */
    function setKeyExpiration(address publicKey, uint256 expiresAt) external onlyBridge {
        if (!sequencerKeys.contains(publicKey) && !validatorKeys.contains(publicKey)) {
            revert InvalidPublicKey(publicKey);
        }
        keyExpiration[publicKey] = expiresAt;
        emit KeyExpirationUpdated(publicKey, expiresAt);
    }

    /**
     * @notice Revokes all registered keys
     * @dev WARNING: May run out of gas if many keys are registered.
     */
    function revokeAllKeys() external onlyBridge {
        address[] memory sKeys = sequencerKeys.values();
        for (uint256 i = 0; i < sKeys.length; i++) {
            sequencerKeys.remove(sKeys[i]);
            delete keyExpiration[sKeys[i]];
        }

        address[] memory vKeys = validatorKeys.values();
        for (uint256 i = 0; i < vKeys.length; i++) {
            validatorKeys.remove(vKeys[i]);
            delete keyExpiration[vKeys[i]];
        }

        emit KeysRevoked();
    }

    /**
     * @notice Updates the attestation verifier
     * @dev Does NOT revoke existing keys - call revokeAllKeys separately if needed
     * @param _attestationVerifier The new attestation verifier contract
     */
    function updateAttestationVerifier(IAttestationVerifier _attestationVerifier) external onlyBridge {
        address oldVerifier = address(attestationVerifier);
        attestationVerifier = _attestationVerifier;
        emit AttestationVerifierUpdated(oldVerifier, address(_attestationVerifier));
    }

    /*//////////////////////////////////////////////////////////////
                            VIEW FUNCTIONS
    //////////////////////////////////////////////////////////////*/

    /**
     * @notice Gets all currently valid keys of a given type
     * @param keyType The type of keys to retrieve
     * @return Array of valid key addresses
     */
    function getKeys(KeyType keyType) external view override returns (address[] memory) {
        return _getKeySet(keyType).values();
    }

    /**
     * @notice Gets the count of valid keys of a given type
     * @param keyType The type of keys to count
     * @return Number of currently valid keys
     */
    function keyCount(KeyType keyType) external view override returns (uint256) {
        return _getKeySet(keyType).length();
    }

    /**
     * @notice Checks if a key is pending approval
     * @param keyType The type of key (Sequencer or Validator)
     * @param publicKey The key to check
     * @return True if pending
     */
    function isKeyPending(KeyType keyType, address publicKey) external view override returns (bool) {
        return _getPendingKeys(keyType, publicKey);
    }
}
