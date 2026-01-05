// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {EnumerableSet} from "@openzeppelin/contracts/utils/structs/EnumerableSet.sol";
import {ECDSA} from "@openzeppelin/contracts/utils/cryptography/ECDSA.sol";
import {MessageHashUtils} from "@openzeppelin/contracts/utils/cryptography/MessageHashUtils.sol";
import {ITeeKeyManager} from "src/interfaces/ITeeKeyManager.sol";
import {IAttestationVerifier} from "src/interfaces/IAttestationVerifier.sol";

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
    event SequencerKeyAdded(address indexed key, uint256 expiresAt);
    event ValidatorKeyAdded(address indexed key, uint256 expiresAt);
    event SequencerKeyPending(address indexed key);
    event ValidatorKeyPending(address indexed key);
    event KeyRemoved(address indexed key);
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
                            KEY VALIDATION
    //////////////////////////////////////////////////////////////*/

    /**
     * @notice Checks if a sequencer key is valid
     * @dev Reverts with InvalidPublicKey if key not registered, KeyExpired if expired
     * @param publicKey The address to check
     * @return True if the key is valid
     */
    function isSequencerKeyValid(address publicKey) external view override returns (bool) {
        if (!sequencerKeys.contains(publicKey)) {
            revert InvalidPublicKey(publicKey);
        }
        uint256 expiry = keyExpiration[publicKey];
        if (expiry != 0 && block.timestamp > expiry) {
            revert KeyExpired(publicKey);
        }
        return true;
    }

    /**
     * @notice Checks if a validator key is valid
     * @dev Reverts with InvalidPublicKey if key not registered, KeyExpired if expired
     * @param publicKey The address to check
     * @return True if the key is valid
     */
    function isValidatorKeyValid(address publicKey) external view override returns (bool) {
        if (!validatorKeys.contains(publicKey)) {
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
     * @notice Registers a sequencer key (called by Bridge)
     * @dev If registration is open, key is added directly. Otherwise, goes to pending.
     * @param publicValues The encoded public values from the attestation
     * @param proofBytes The SP1 proof bytes
     * @param requiresApproval Whether the key needs owner approval
     * @param expiresAt Expiration timestamp (0 = never expires)
     * @return publicKey The registered key address
     */
    function addSequencerKey(
        bytes calldata publicValues,
        bytes calldata proofBytes,
        bool requiresApproval,
        uint256 expiresAt
    ) external onlyBridge returns (address publicKey) {
        publicKey = attestationVerifier.verifyAttestationProof(publicValues, proofBytes);

        if (sequencerKeys.contains(publicKey)) revert KeyAlreadyExists(publicKey);

        if (requiresApproval) {
            pendingSequencerKeys[publicKey] = true;
            emit SequencerKeyPending(publicKey);
        } else {
            sequencerKeys.add(publicKey);
            keyExpiration[publicKey] = expiresAt;
            emit SequencerKeyAdded(publicKey, expiresAt);
        }
    }

    /**
     * @notice Registers a validator key (called by Bridge)
     * @dev If registration is open, key is added directly. Otherwise, goes to pending.
     * @param publicValues The encoded public values from the attestation
     * @param proofBytes The SP1 proof bytes
     * @param requiresApproval Whether the key needs owner approval
     * @param expiresAt Expiration timestamp (0 = never expires)
     * @return publicKey The registered key address
     */
    function addValidatorKey(
        bytes calldata publicValues,
        bytes calldata proofBytes,
        bool requiresApproval,
        uint256 expiresAt
    ) external onlyBridge returns (address publicKey) {
        publicKey = attestationVerifier.verifyAttestationProof(publicValues, proofBytes);

        if (validatorKeys.contains(publicKey)) revert KeyAlreadyExists(publicKey);

        if (requiresApproval) {
            pendingValidatorKeys[publicKey] = true;
            emit ValidatorKeyPending(publicKey);
        } else {
            validatorKeys.add(publicKey);
            keyExpiration[publicKey] = expiresAt;
            emit ValidatorKeyAdded(publicKey, expiresAt);
        }
    }

    /**
     * @notice Registers a sequencer key via signature (for keys without gas)
     * @param publicValues The encoded public values from the attestation
     * @param proofBytes The SP1 proof bytes
     * @param deadline Timestamp after which the signature expires
     * @param signature EIP-712 signature from the TEE key
     * @param requiresApproval Whether the key needs owner approval
     * @param expiresAt Expiration timestamp (0 = never expires)
     * @return publicKey The registered key address
     */
    function addSequencerKeyWithSignature(
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

        if (sequencerKeys.contains(publicKey)) revert KeyAlreadyExists(publicKey);

        if (requiresApproval) {
            pendingSequencerKeys[publicKey] = true;
            emit SequencerKeyPending(publicKey);
        } else {
            sequencerKeys.add(publicKey);
            keyExpiration[publicKey] = expiresAt;
            emit SequencerKeyAdded(publicKey, expiresAt);
        }
    }

    /**
     * @notice Registers a validator key via signature (for keys without gas)
     * @param publicValues The encoded public values from the attestation
     * @param proofBytes The SP1 proof bytes
     * @param deadline Timestamp after which the signature expires
     * @param signature EIP-712 signature from the TEE key
     * @param requiresApproval Whether the key needs owner approval
     * @param expiresAt Expiration timestamp (0 = never expires)
     * @return publicKey The registered key address
     */
    function addValidatorKeyWithSignature(
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

        if (validatorKeys.contains(publicKey)) revert KeyAlreadyExists(publicKey);

        if (requiresApproval) {
            pendingValidatorKeys[publicKey] = true;
            emit ValidatorKeyPending(publicKey);
        } else {
            validatorKeys.add(publicKey);
            keyExpiration[publicKey] = expiresAt;
            emit ValidatorKeyAdded(publicKey, expiresAt);
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
     * @notice Approves a pending sequencer key
     * @param publicKey The pending key to approve
     * @param expiresAt Expiration timestamp (0 = never expires)
     */
    function approveSequencerKey(address publicKey, uint256 expiresAt) external onlyBridge {
        if (!pendingSequencerKeys[publicKey]) revert KeyNotPending(publicKey);

        delete pendingSequencerKeys[publicKey];
        sequencerKeys.add(publicKey);
        keyExpiration[publicKey] = expiresAt;
        emit SequencerKeyAdded(publicKey, expiresAt);
    }

    /**
     * @notice Approves a pending validator key
     * @param publicKey The pending key to approve
     * @param expiresAt Expiration timestamp (0 = never expires)
     */
    function approveValidatorKey(address publicKey, uint256 expiresAt) external onlyBridge {
        if (!pendingValidatorKeys[publicKey]) revert KeyNotPending(publicKey);

        delete pendingValidatorKeys[publicKey];
        validatorKeys.add(publicKey);
        keyExpiration[publicKey] = expiresAt;
        emit ValidatorKeyAdded(publicKey, expiresAt);
    }

    /**
     * @notice Rejects a pending sequencer key
     * @param publicKey The pending key to reject
     */
    function rejectSequencerKey(address publicKey) external onlyBridge {
        if (!pendingSequencerKeys[publicKey]) revert KeyNotPending(publicKey);
        delete pendingSequencerKeys[publicKey];
    }

    /**
     * @notice Rejects a pending validator key
     * @param publicKey The pending key to reject
     */
    function rejectValidatorKey(address publicKey) external onlyBridge {
        if (!pendingValidatorKeys[publicKey]) revert KeyNotPending(publicKey);
        delete pendingValidatorKeys[publicKey];
    }

    /*//////////////////////////////////////////////////////////////
                            KEY MANAGEMENT
    //////////////////////////////////////////////////////////////*/

    /**
     * @notice Removes a sequencer key
     * @param publicKey The key to remove
     */
    function removeSequencerKey(address publicKey) external onlyBridge {
        if (!sequencerKeys.remove(publicKey)) revert InvalidPublicKey(publicKey);
        delete keyExpiration[publicKey];
        emit KeyRemoved(publicKey);
    }

    /**
     * @notice Removes a validator key
     * @param publicKey The key to remove
     */
    function removeValidatorKey(address publicKey) external onlyBridge {
        if (!validatorKeys.remove(publicKey)) revert InvalidPublicKey(publicKey);
        delete keyExpiration[publicKey];
        emit KeyRemoved(publicKey);
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
     * @notice Gets all currently valid sequencer keys
     * @return Array of valid sequencer key addresses
     */
    function getSequencerKeys() external view returns (address[] memory) {
        return sequencerKeys.values();
    }

    /**
     * @notice Gets all currently valid validator keys
     * @return Array of valid validator key addresses
     */
    function getValidatorKeys() external view override returns (address[] memory) {
        return validatorKeys.values();
    }

    /**
     * @notice Gets the count of valid sequencer keys
     * @return Number of currently valid sequencer keys
     */
    function sequencerKeyCount() external view returns (uint256) {
        return sequencerKeys.length();
    }

    /**
     * @notice Gets the count of valid validator keys
     * @return Number of currently valid validator keys
     */
    function validatorKeyCount() external view returns (uint256) {
        return validatorKeys.length();
    }

    /**
     * @notice Checks if a key is a pending sequencer key
     * @param publicKey The key to check
     * @return True if pending
     */
    function isSequencerKeyPending(address publicKey) external view returns (bool) {
        return pendingSequencerKeys[publicKey];
    }

    /**
     * @notice Checks if a key is a pending validator key
     * @param publicKey The key to check
     * @return True if pending
     */
    function isValidatorKeyPending(address publicKey) external view returns (bool) {
        return pendingValidatorKeys[publicKey];
    }
}
