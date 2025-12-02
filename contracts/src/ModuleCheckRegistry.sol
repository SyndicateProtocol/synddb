// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {IModuleCheck} from "src/interfaces/IModuleCheck.sol";
import {ProcessingStage, SequencerSignature} from "src/types/DataTypes.sol";
import {IModuleCheckRegistry} from "src/interfaces/IModuleCheckRegistry.sol";
import {IValidatorSigningAndQuery} from "src/interfaces/IValidatorSigningAndQuery.sol";
import {AccessControl} from "@openzeppelin/contracts/access/AccessControl.sol";
import {EnumerableSet} from "@openzeppelin/contracts/utils/structs/EnumerableSet.sol";
import {ECDSA} from "@openzeppelin/contracts/utils/cryptography/ECDSA.sol";
import {MessageHashUtils} from "@openzeppelin/contracts/utils/cryptography/MessageHashUtils.sol";

/**
 * @title ModuleCheckRegistry
 * @notice Abstract contract that manages validation modules and validator signatures
 * @dev Provides role-based access control for sequencers and validators, and manages pre/post execution validation modules
 */
abstract contract ModuleCheckRegistry is IModuleCheckRegistry, IValidatorSigningAndQuery, AccessControl {
    using EnumerableSet for EnumerableSet.AddressSet;

    bytes32 public constant SEQUENCER_ROLE = keccak256("SEQUENCER_ROLE");
    bytes32 public constant VALIDATOR_ROLE = keccak256("VALIDATOR_ROLE");

    EnumerableSet.AddressSet private preModules;
    EnumerableSet.AddressSet private postModules;

    mapping(bytes32 messageId => mapping(address validator => bool hasSigned)) public validatorSignatures;

    /**
     * @notice Emitted when a pre-execution module is added to the registry
     * @param module Address of the added module
     */
    event PreModuleAdded(address indexed module);

    /**
     * @notice Emitted when a post-execution module is added to the registry
     * @param module Address of the added module
     */
    event PostModuleAdded(address indexed module);

    /**
     * @notice Emitted when a pre-execution module is removed from the registry
     * @param module Address of the removed module
     */
    event PreModuleRemoved(address indexed module);

    /**
     * @notice Emitted when a post-execution module is removed from the registry
     * @param module Address of the removed module
     */
    event PostModuleRemoved(address indexed module);

    error InvalidModuleAddress();
    error ModuleAlreadyExists();
    error ModuleDoesNotExist();
    error InvalidPreExecutionStage(ProcessingStage stage);
    error InvalidPostExecutionStage(ProcessingStage stage);
    error ModuleCheckFailed(address module, ProcessingStage stage);
    error InvalidValidatorSignature();

    /**
     * @notice Initializes the contract with an admin address
     * @param admin Address to be granted the DEFAULT_ADMIN_ROLE
     */
    constructor(address admin) {
        _grantRole(DEFAULT_ADMIN_ROLE, admin);
    }

    /**
     * @notice Adds a validation module to run before message execution
     * @dev Only callable by DEFAULT_ADMIN_ROLE
     * @param module Address of the module implementing IModuleCheck
     */
    function addPreModule(address module) external virtual onlyRole(DEFAULT_ADMIN_ROLE) {
        if (module == address(0)) revert InvalidModuleAddress();
        if (!preModules.add(module)) revert ModuleAlreadyExists();
        emit PreModuleAdded(module);
    }

    /**
     * @notice Adds a validation module to run after message execution
     * @dev Only callable by DEFAULT_ADMIN_ROLE
     * @param module Address of the module implementing IModuleCheck
     */
    function addPostModule(address module) external virtual onlyRole(DEFAULT_ADMIN_ROLE) {
        if (module == address(0)) revert InvalidModuleAddress();
        if (!postModules.add(module)) revert ModuleAlreadyExists();
        emit PostModuleAdded(module);
    }

    /**
     * @notice Removes a pre-execution validation module
     * @dev Only callable by DEFAULT_ADMIN_ROLE
     * @param module Address of the module to remove
     */
    function removePreModule(address module) external virtual onlyRole(DEFAULT_ADMIN_ROLE) {
        if (module == address(0)) revert InvalidModuleAddress();
        if (!preModules.remove(module)) revert ModuleDoesNotExist();
        emit PreModuleRemoved(module);
    }

    /**
     * @notice Removes a post-execution validation module
     * @dev Only callable by DEFAULT_ADMIN_ROLE
     * @param module Address of the module to remove
     */
    function removePostModule(address module) external virtual onlyRole(DEFAULT_ADMIN_ROLE) {
        if (module == address(0)) revert InvalidModuleAddress();
        if (!postModules.remove(module)) revert ModuleDoesNotExist();
        emit PostModuleRemoved(module);
    }

    /**
     * @notice Gets all registered pre-execution modules
     * @return Array of module addresses that run before execution
     */
    function getPreModules() external view virtual returns (address[] memory) {
        return preModules.values();
    }

    /**
     * @notice Gets all registered post-execution modules
     * @return Array of module addresses that run after execution
     */
    function getPostModules() external view virtual returns (address[] memory) {
        return postModules.values();
    }

    /**
     * @notice Internal function to validate a message against a set of modules
     * @dev Iterates through all modules and reverts if any check fails
     * @param messageId Unique identifier of the message
     * @param modules Set of module addresses to validate against
     * @param stage Current processing stage
     * @param payload Encoded function call data
     * @param sequencerSignature Signature from the trusted sequencer
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
     * @dev Ensures stage is PreExecution before validating
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
     * @dev Ensures stage is PostExecution before validating
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

    /**
     * @notice Allows a validator to directly sign a message
     * @dev Uses msg.sender as the validator address (direct call pattern)
     * @dev Only callable by addresses with VALIDATOR_ROLE
     * @param messageId The unique identifier of the message to sign
     */
    function signMessage(bytes32 messageId) external onlyRole(VALIDATOR_ROLE) {
        // Direct validator call pattern - validator calls via msg.sender
        validatorSignatures[messageId][msg.sender] = true;
        emit MessageSigned(messageId, msg.sender);
    }

    /**
     * @notice Allows anyone to submit a validator's signature for a message
     * @dev Verifies the signature cryptographically (relayer pattern)
     * @param messageId The unique identifier of the message to sign
     * @param signature The ECDSA signature from an authorized validator
     */
    function signMessageWithSignature(bytes32 messageId, bytes calldata signature) public {
        // Relayer pattern - relayer submits validator's signature
        bytes32 messageHash = MessageHashUtils.toEthSignedMessageHash(messageId);
        address validator = ECDSA.recover(messageHash, signature);

        if (!hasRole(VALIDATOR_ROLE, validator)) {
            revert ValidatorNotAuthorized();
        }

        validatorSignatures[messageId][validator] = true;
        emit MessageSigned(messageId, validator);
    }

    /**
     * @notice Counts how many of the provided validators have signed a specific message
     * @param messageId The unique identifier of the message
     * @param validators Array of validator addresses to check
     * @return uint256 The number of validators who have signed the message
     */
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
