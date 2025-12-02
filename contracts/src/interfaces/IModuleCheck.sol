// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;
import {ProcessingStage, SequencerSignature} from "src/types/DataTypes.sol";

/**
 * @title IModuleCheck
 * @notice Interface for validation modules that enforce custom rules during message processing
 * @dev Modules can validate state before or after message execution to enforce constraints like supply limits, signature thresholds, etc.
 */
interface IModuleCheck {
    /**
     * @notice Validates a message at a specific processing stage
     * @dev Called by the bridge during pre-execution or post-execution phases
     * @param messageId Unique identifier of the message being processed
     * @param stage Current processing stage (PreExecution or PostExecution)
     * @param payload The encoded function call data
     * @param sequencerSignature Signature from the trusted sequencer
     * @return bool True if validation passes, false otherwise
     */
    function check(
        bytes32 messageId,
        ProcessingStage stage,
        bytes memory payload,
        SequencerSignature memory sequencerSignature
    ) external returns (bool);
}
