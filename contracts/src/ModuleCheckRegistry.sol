// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {IModuleCheck} from "src/interfaces/IModuleCheck.sol";
import {ProcessingStage, ValidatorSignatures} from "src/types/DataTypes.sol";
import {IModuleCheckResgistry} from "src/interfaces/IModuleCheckResgistry.sol";
import {Ownable} from "@openzeppelin/contracts/access/Ownable.sol";
import {EnumerableSet} from "@openzeppelin/contracts/utils/structs/EnumerableSet.sol";

abstract contract ModuleCheckRegistry is IModuleCheckResgistry, Ownable {
    using EnumerableSet for EnumerableSet.AddressSet;

    EnumerableSet.AddressSet private preModules;
    EnumerableSet.AddressSet private postModules;

    event PreModuleAdded(address indexed module);
    event PostModuleAdded(address indexed module);
    event PreModuleRemoved(address indexed module);
    event PostModuleRemoved(address indexed module);

    error InvalidModuleAddress();
    error ModuleAlreadyExists();
    error ModuleDoesNotExist();
    error InvalidPreExecutionStage(ProcessingStage stage);
    error InvalidPostExecutionStage(ProcessingStage stage);
    error ModuleCheckFailed(address module, ProcessingStage stage);

    constructor() Ownable(msg.sender) {}

    function addPreModule(address module) external virtual onlyOwner {
        if (module == address(0)) revert InvalidModuleAddress();
        if (!preModules.add(module)) revert ModuleAlreadyExists();
        emit PreModuleAdded(module);
    }

    function addPostModule(address module) external virtual onlyOwner {
        if (module == address(0)) revert InvalidModuleAddress();
        if (!postModules.add(module)) revert ModuleAlreadyExists();
        emit PostModuleAdded(module);
    }

    function removePreModule(address module) external virtual onlyOwner {
        if (module == address(0)) revert InvalidModuleAddress();
        if (!preModules.remove(module)) revert ModuleDoesNotExist();
        emit PreModuleRemoved(module);
    }

    function removePostModule(address module) external virtual onlyOwner {
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
        ValidatorSignatures memory executionSignatures
    ) internal returns (bool) {
        uint256 length = modules.length();
        for (uint256 i = 0; i < length; i++) {
            address moduleAddress = modules.at(i);
            if (!IModuleCheck(moduleAddress).check(stage, payload, executionSignatures)) {
                revert ModuleCheckFailed(moduleAddress, stage);
            }
        }
        return true;
    }

    function _validatePreModules(
        ProcessingStage stage,
        bytes memory payload,
        ValidatorSignatures memory executionSignatures
    ) internal returns (bool) {
        if (stage != ProcessingStage.PreExecution) {
            revert InvalidPreExecutionStage(stage);
        }
        return _validateModules(preModules, stage, payload, executionSignatures);
    }

    function _validatePostModules(
        ProcessingStage stage,
        bytes memory payload,
        ValidatorSignatures memory executionSignatures
    ) internal returns (bool) {
        if (stage != ProcessingStage.PostExecution) {
            revert InvalidPostExecutionStage(stage);
        }
        return _validateModules(postModules, stage, payload, executionSignatures);
    }
}
