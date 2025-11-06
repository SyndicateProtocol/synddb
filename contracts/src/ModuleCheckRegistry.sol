// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {IModuleCheck} from "src/interfaces/IModuleCheck.sol";
import {ProcessingStage, SequencerSignature} from "src/types/DataTypes.sol";
import {IModuleCheckResgistry} from "src/interfaces/IModuleCheckResgistry.sol";
import {AccessControl} from "@openzeppelin/contracts/access/AccessControl.sol";
import {EnumerableSet} from "@openzeppelin/contracts/utils/structs/EnumerableSet.sol";
import {ECDSA} from "@openzeppelin/contracts/utils/cryptography/ECDSA.sol";
import {MessageHashUtils} from "@openzeppelin/contracts/utils/cryptography/MessageHashUtils.sol";

abstract contract ModuleCheckRegistry is IModuleCheckResgistry, AccessControl {
    using EnumerableSet for EnumerableSet.AddressSet;

    bytes32 public constant SEQUENCER_ROLE = keccak256("SEQUENCER_ROLE");
    bytes32 public constant VALIDATOR_ROLE = keccak256("VALIDATOR_ROLE");

    EnumerableSet.AddressSet private preModules;
    EnumerableSet.AddressSet private postModules;

    mapping(bytes32 messageId => mapping(address validator => bool hasSigned)) public validatorSignatures;

    event PreModuleAdded(address indexed module);
    event PostModuleAdded(address indexed module);
    event PreModuleRemoved(address indexed module);
    event PostModuleRemoved(address indexed module);
    event MessageSigned(bytes32 indexed messageId, address indexed validator);

    error InvalidModuleAddress();
    error ModuleAlreadyExists();
    error ModuleDoesNotExist();
    error InvalidPreExecutionStage(ProcessingStage stage);
    error InvalidPostExecutionStage(ProcessingStage stage);
    error ModuleCheckFailed(address module, ProcessingStage stage);
    error InvalidValidatorSignature();
    error ValidatorNotAuthorized();

    constructor(address admin) {
        _grantRole(DEFAULT_ADMIN_ROLE, admin);
    }

    function addPreModule(address module) external virtual onlyRole(DEFAULT_ADMIN_ROLE) {
        if (module == address(0)) revert InvalidModuleAddress();
        if (!preModules.add(module)) revert ModuleAlreadyExists();
        emit PreModuleAdded(module);
    }

    function addPostModule(address module) external virtual onlyRole(DEFAULT_ADMIN_ROLE) {
        if (module == address(0)) revert InvalidModuleAddress();
        if (!postModules.add(module)) revert ModuleAlreadyExists();
        emit PostModuleAdded(module);
    }

    function removePreModule(address module) external virtual onlyRole(DEFAULT_ADMIN_ROLE) {
        if (module == address(0)) revert InvalidModuleAddress();
        if (!preModules.remove(module)) revert ModuleDoesNotExist();
        emit PreModuleRemoved(module);
    }

    function removePostModule(address module) external virtual onlyRole(DEFAULT_ADMIN_ROLE) {
        if (module == address(0)) revert InvalidModuleAddress();
        if (!postModules.remove(module)) revert ModuleDoesNotExist();
        emit PostModuleRemoved(module);
    }

    function getPreModules() external view virtual returns (address[] memory) {
        return preModules.values();
    }

    function getPostModules() external view virtual returns (address[] memory) {
        return postModules.values();
    }

    function _validateModules(
        EnumerableSet.AddressSet storage modules,
        ProcessingStage stage,
        bytes memory payload,
        SequencerSignature memory sequencerSignature
    ) internal returns (bool) {
        uint256 length = modules.length();
        for (uint256 i = 0; i < length; i++) {
            address moduleAddress = modules.at(i);
            if (!IModuleCheck(moduleAddress).check(stage, payload, sequencerSignature)) {
                revert ModuleCheckFailed(moduleAddress, stage);
            }
        }
        return true;
    }

    function _validatePreModules(
        ProcessingStage stage,
        bytes memory payload,
        SequencerSignature memory sequencerSignature
    ) internal returns (bool) {
        if (stage != ProcessingStage.PreExecution) {
            revert InvalidPreExecutionStage(stage);
        }
        return _validateModules(preModules, stage, payload, sequencerSignature);
    }

    function _validatePostModules(
        ProcessingStage stage,
        bytes memory payload,
        SequencerSignature memory sequencerSignature
    ) internal returns (bool) {
        if (stage != ProcessingStage.PostExecution) {
            revert InvalidPostExecutionStage(stage);
        }
        return _validateModules(postModules, stage, payload, sequencerSignature);
    }

    /*//////////////////////////////////////////////////////////////
                                 VALIDATORS SIGNING
    //////////////////////////////////////////////////////////////*/

    function signMessage(bytes32 messageId) external onlyRole(VALIDATOR_ROLE) {
        // Direct validator call pattern - validator calls via msg.sender
        validatorSignatures[messageId][msg.sender] = true;
        emit MessageSigned(messageId, msg.sender);
    }

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
}
