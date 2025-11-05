// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {IModuleValidator} from "./interfaces/IModuleValidator.sol";
import {IBridge} from "./interfaces/IBridge.sol";
import {Ownable} from "@openzeppelin/contracts/access/Ownable.sol";
import {EnumerableSet} from "@openzeppelin/contracts/utils/structs/EnumerableSet.sol";

abstract contract ModuleValidator is Ownable {
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
    error InvalidPreExecutionStage(IBridge.ProcessingStage stage);
    error InvalidPostExecutionStage(IBridge.ProcessingStage stage);
    error ModuleCheckFailed(address module, IBridge.ProcessingStage stage);

    constructor() Ownable(msg.sender) {}

    function addPreModule(address module) external onlyOwner {
        if (module == address(0)) revert InvalidModuleAddress();
        if (!preModules.add(module)) revert ModuleAlreadyExists();
        emit PreModuleAdded(module);
    }

    function addPostModule(address module) external onlyOwner {
        if (module == address(0)) revert InvalidModuleAddress();
        if (!postModules.add(module)) revert ModuleAlreadyExists();
        emit PostModuleAdded(module);
    }

    function removePreModule(address module) external onlyOwner {
        if (module == address(0)) revert InvalidModuleAddress();
        if (!preModules.remove(module)) revert ModuleDoesNotExist();
        emit PreModuleRemoved(module);
    }

    function removePostModule(address module) external onlyOwner {
        if (module == address(0)) revert InvalidModuleAddress();
        if (!postModules.remove(module)) revert ModuleDoesNotExist();
        emit PostModuleRemoved(module);
    }

    function getPreModules() external view returns (address[] memory) {
        return preModules.values();
    }

    function getPostModules() external view returns (address[] memory) {
        return postModules.values();
    }

    function _validateModules(
        EnumerableSet.AddressSet storage modules,
        IBridge.ProcessingStage stage,
        bytes calldata payload,
        IBridge.ValidatorSignatures calldata executionSigs
    ) internal returns (bool) {
        uint256 length = modules.length();
        for (uint256 i = 0; i < length; i++) {
            address moduleAddress = modules.at(i);
            if (!IModuleValidator(moduleAddress).validate(stage, payload, executionSigs)) {
                revert ModuleCheckFailed(moduleAddress, stage);
            }
        }
        return true;
    }

    function _validatePreModules(
        IBridge.ProcessingStage stage,
        bytes calldata payload,
        IBridge.ValidatorSignatures calldata executionSigs
    ) internal returns (bool) {
        if (stage != IBridge.ProcessingStage.PreExecution) {
            revert InvalidPreExecutionStage(stage);
        }
        return _validateModules(preModules, stage, payload, executionSigs);
    }

    function _validatePostModules(
        IBridge.ProcessingStage stage,
        bytes calldata payload,
        IBridge.ValidatorSignatures calldata executionSigs
    ) internal returns (bool) {
        if (stage != IBridge.ProcessingStage.PostExecution) {
            revert InvalidPostExecutionStage(stage);
        }
        return _validateModules(postModules, stage, payload, executionSigs);
    }
}
