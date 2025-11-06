// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;
import {ProcessingStage, SequencerSignature} from "src/types/DataTypes.sol";

interface IModuleCheck {
    function check(
        bytes32 messageId,
        ProcessingStage stage,
        bytes memory payload,
        SequencerSignature memory sequencerSignature
    ) external returns (bool);
}
