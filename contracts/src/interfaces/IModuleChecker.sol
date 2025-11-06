// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {IBridge} from "./IBridge.sol";

interface IModuleChecker {
    function check(
        IBridge.ProcessingStage stage,
        bytes memory payload,
        IBridge.ValidatorSignatures memory executionSigs
    ) external returns (bool);
}
