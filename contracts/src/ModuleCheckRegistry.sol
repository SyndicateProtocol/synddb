// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {IModuleCheck} from "src/interfaces/IModuleCheck.sol";
import {ProcessingStage, SequencerSignature} from "src/types/DataTypes.sol";
import {IModuleCheckRegistry} from "src/interfaces/IModuleCheckRegistry.sol";
import {IValidatorSigningAndQuery} from "src/interfaces/IValidatorSigningAndQuery.sol";
import {ITeeKeyManager} from "src/interfaces/ITeeKeyManager.sol";
import {Ownable2Step, Ownable} from "@openzeppelin/contracts/access/Ownable2Step.sol";
import {EnumerableSet} from "@openzeppelin/contracts/utils/structs/EnumerableSet.sol";
import {ECDSA} from "@openzeppelin/contracts/utils/cryptography/ECDSA.sol";
import {MessageHashUtils} from "@openzeppelin/contracts/utils/cryptography/MessageHashUtils.sol";

/**
 * @title ModuleCheckRegistry
 * @notice Abstract contract that manages validation modules, validator signatures, and sequencer signatures
 * @dev Inherited by Bridge.sol. Provides owner-based access control for admin operations,
 *      role-based access for message initializers, and manages pre/post execution validation modules.
 *      Validator authorization is delegated to TeeKeyManager (TEE attestation-based).
 */
abstract contract ModuleCheckRegistry is IModuleCheckRegistry, IValidatorSigningAndQuery, Ownable2Step {
    using EnumerableSet for EnumerableSet.AddressSet;

    EnumerableSet.AddressSet private preModules;
    EnumerableSet.AddressSet private postModules;

    /// @notice Addresses authorized to initialize messages
    mapping(address account => bool isAuthorized) public messageInitializers;

    mapping(bytes32 messageId => mapping(address validator => bool hasSigned)) public validatorSignatures;
    mapping(bytes32 messageId => SequencerSignature signature) public sequencerSignatures;

    /// @notice Reference to the TEE key manager for validator authorization
    ITeeKeyManager public teeKeyManager;

    event PreModuleAdded(address indexed module);
    event PostModuleAdded(address indexed module);
    event PreModuleRemoved(address indexed module);
    event PostModuleRemoved(address indexed module);
    event MessageInitializerUpdated(address indexed account, bool authorized);

    error InvalidModuleAddress();
    error ModuleAlreadyExists();
    error ModuleDoesNotExist();
    error InvalidPreExecutionStage(ProcessingStage stage);
    error InvalidPostExecutionStage(ProcessingStage stage);
    error ModuleCheckFailed(address module, ProcessingStage stage);
    error InvalidValidatorSignature();
    error NotMessageInitializer();

    modifier onlyMessageInitializer() {
        if (!messageInitializers[msg.sender]) revert NotMessageInitializer();
        _;
    }

    /**
     * @notice Initializes the contract with an owner address
     * @dev Owner has the ability to:
     *      - Add/remove validation modules
     *      - Grant/revoke message initializer permissions
     *      - Manage TEE keys via TeeKeyManager
     * @param _owner Address to be granted ownership
     */
    constructor(address _owner) Ownable(_owner) {}

    /// @inheritdoc IModuleCheckRegistry
    function addPreModule(address module) external virtual onlyOwner {
        if (module == address(0)) revert InvalidModuleAddress();
        if (!preModules.add(module)) revert ModuleAlreadyExists();
        emit PreModuleAdded(module);
    }

    /// @inheritdoc IModuleCheckRegistry
    function addPostModule(address module) external virtual onlyOwner {
        if (module == address(0)) revert InvalidModuleAddress();
        if (!postModules.add(module)) revert ModuleAlreadyExists();
        emit PostModuleAdded(module);
    }

    /// @inheritdoc IModuleCheckRegistry
    function removePreModule(address module) external virtual onlyOwner {
        if (module == address(0)) revert InvalidModuleAddress();
        if (!preModules.remove(module)) revert ModuleDoesNotExist();
        emit PreModuleRemoved(module);
    }

    /// @inheritdoc IModuleCheckRegistry
    function removePostModule(address module) external virtual onlyOwner {
        if (module == address(0)) revert InvalidModuleAddress();
        if (!postModules.remove(module)) revert ModuleDoesNotExist();
        emit PostModuleRemoved(module);
    }

    /**
     * @notice Grants or revokes message initializer permission
     * @dev Only callable by owner
     * @param account Address to update
     * @param authorized Whether the account should be authorized
     */
    function setMessageInitializer(address account, bool authorized) external onlyOwner {
        messageInitializers[account] = authorized;
        emit MessageInitializerUpdated(account, authorized);
    }

    /// @inheritdoc IModuleCheckRegistry
    function getPreModules() external view virtual returns (address[] memory) {
        return preModules.values();
    }

    /// @inheritdoc IModuleCheckRegistry
    function getPostModules() external view virtual returns (address[] memory) {
        return postModules.values();
    }

    /**
     * @notice Internal function to validate a message against a set of modules
     * @dev Iterates through all modules and reverts if any check fails.
     *      WARNING: Ensure the total gas cost of all modules does NOT exceed the block gas limit.
     * @param messageId Unique identifier of the message
     * @param modules Set of module addresses to validate against
     * @param stage Current processing stage
     * @param payload Encoded function call data
     * @param sequencerSignature Signature from the trusted TEE sequencer
     * @return bool True if all modules pass validation
     */
    function _validateModules(
        bytes32 messageId,
        EnumerableSet.AddressSet storage modules,
        ProcessingStage stage,
        bytes memory payload,
        SequencerSignature memory sequencerSignature
    ) internal returns (bool) {
        uint256 length = modules.length();
        for (uint256 i = 0; i < length; i++) {
            address moduleAddress = modules.at(i);
            if (!IModuleCheck(moduleAddress).check(messageId, stage, payload, sequencerSignature)) {
                revert ModuleCheckFailed(moduleAddress, stage);
            }
        }
        return true;
    }

    /**
     * @notice Validates all pre-execution modules for a message
     * @param messageId Unique identifier of the message
     * @param stage Must be ProcessingStage.PreExecution
     * @param payload Encoded function call data
     * @param sequencerSignature Signature from the trusted sequencer
     * @return bool True if all pre-modules pass validation
     */
    function _validatePreModules(
        bytes32 messageId,
        ProcessingStage stage,
        bytes memory payload,
        SequencerSignature memory sequencerSignature
    ) internal returns (bool) {
        if (stage != ProcessingStage.PreExecution) {
            revert InvalidPreExecutionStage(stage);
        }
        return _validateModules(messageId, preModules, stage, payload, sequencerSignature);
    }

    /**
     * @notice Validates all post-execution modules for a message
     * @param messageId Unique identifier of the message
     * @param stage Must be ProcessingStage.PostExecution
     * @param payload Encoded function call data
     * @param sequencerSignature Signature from the trusted sequencer
     * @return bool True if all post-modules pass validation
     */
    function _validatePostModules(
        bytes32 messageId,
        ProcessingStage stage,
        bytes memory payload,
        SequencerSignature memory sequencerSignature
    ) internal returns (bool) {
        if (stage != ProcessingStage.PostExecution) {
            revert InvalidPostExecutionStage(stage);
        }
        return _validateModules(messageId, postModules, stage, payload, sequencerSignature);
    }

    /*//////////////////////////////////////////////////////////////
                             VALIDATORS SIGNING
    //////////////////////////////////////////////////////////////*/

    /// @inheritdoc IValidatorSigningAndQuery
    function signMessage(bytes32 messageId) external {
        // Validator must be registered in TeeKeyManager
        // This will revert if not a valid validator key
        teeKeyManager.isValidatorKeyValid(msg.sender);

        validatorSignatures[messageId][msg.sender] = true;
        emit MessageSigned(messageId, msg.sender, msg.sender);
    }

    /// @inheritdoc IValidatorSigningAndQuery
    function signMessageWithSignature(bytes32 messageId, bytes calldata signature) public {
        // Relayer pattern - relayer submits validator's signature
        bytes32 messageHash = MessageHashUtils.toEthSignedMessageHash(messageId);
        address validator = ECDSA.recover(messageHash, signature);

        // Validator must be registered in TeeKeyManager
        // This will revert if not a valid validator key
        teeKeyManager.isValidatorKeyValid(validator);

        validatorSignatures[messageId][validator] = true;
        emit MessageSigned(messageId, validator, msg.sender);
    }

    /// @inheritdoc IValidatorSigningAndQuery
    function getValidatorSignatureCount(bytes32 messageId, address[] calldata validators)
        external
        view
        returns (uint256)
    {
        uint256 count = 0;
        for (uint256 i = 0; i < validators.length; i++) {
            if (validatorSignatures[messageId][validators[i]]) {
                count++;
            }
        }
        return count;
    }
}
