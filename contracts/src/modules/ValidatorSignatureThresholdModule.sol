// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {IModuleCheck} from "src/interfaces/IModuleCheck.sol";
import {IValidatorSigningAndQuery} from "src/interfaces/IValidatorSigningAndQuery.sol";
import {ProcessingStage, SequencerSignature} from "src/types/DataTypes.sol";
import {Ownable} from "@openzeppelin/contracts/access/Ownable.sol";

contract ValidatorSignatureThresholdModule is IModuleCheck, Ownable {
    IValidatorSigningAndQuery public immutable BRIDGE;
    address[] public validators;
    uint256 public signatureThreshold;

    error ZeroAddress();
    error EmptyValidatorArray();
    error DuplicateValidator(address validator);
    error ThresholdZero();
    error ThresholdExceedsValidatorCount(uint256 threshold, uint256 validatorCount);

    constructor(address bridge, address[] memory _validators, uint256 _signatureThreshold) Ownable(msg.sender) {
        if (bridge == address(0)) {
            revert ZeroAddress();
        }
        if (_validators.length == 0) {
            revert EmptyValidatorArray();
        }
        if (_signatureThreshold == 0) {
            revert ThresholdZero();
        }
        if (_signatureThreshold > _validators.length) {
            revert ThresholdExceedsValidatorCount(_signatureThreshold, _validators.length);
        }

        // Check for duplicate validators
        for (uint256 i = 0; i < _validators.length; i++) {
            for (uint256 j = i + 1; j < _validators.length; j++) {
                if (_validators[i] == _validators[j]) {
                    revert DuplicateValidator(_validators[i]);
                }
            }
        }

        BRIDGE = IValidatorSigningAndQuery(bridge);
        validators = _validators;
        signatureThreshold = _signatureThreshold;
    }

    function setThreshold(uint256 _signatureThreshold) external onlyOwner {
        if (_signatureThreshold == 0) {
            revert ThresholdZero();
        }
        if (_signatureThreshold > validators.length) {
            revert ThresholdExceedsValidatorCount(_signatureThreshold, validators.length);
        }

        signatureThreshold = _signatureThreshold;
    }

    function check(bytes32 messageId, ProcessingStage stage, bytes memory, SequencerSignature memory)
        external
        view
        returns (bool)
    {
        if (stage == ProcessingStage.PreExecution) {
            uint256 signatureCount = BRIDGE.getValidatorSignatureCount(messageId, validators);

            if (signatureCount < signatureThreshold) {
                return false;
            }

            return true;
        }

        return true;
    }
}
