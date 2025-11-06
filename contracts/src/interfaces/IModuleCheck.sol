// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;
import {ProcessingStage, ValidatorSignatures} from "src/types/DataTypes.sol";

interface IModuleCheck {
    function check(ProcessingStage stage, bytes memory payload, ValidatorSignatures memory executionSignatures)
        external
        returns (bool);
}
