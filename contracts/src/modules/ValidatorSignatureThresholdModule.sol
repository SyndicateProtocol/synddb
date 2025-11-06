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

    constructor(address bridge, address[] memory _validators, uint256 _signatureThreshold) Ownable(msg.sender) {
        BRIDGE = IValidatorSigningAndQuery(bridge);
        validators = _validators;
        signatureThreshold = _signatureThreshold;
    }

    function setThreshold(uint256 _signatureThreshold) external onlyOwner {
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
