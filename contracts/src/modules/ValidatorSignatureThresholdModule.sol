// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {IModuleCheck} from "src/interfaces/IModuleCheck.sol";
import {IValidatorSigningAndQuery} from "src/interfaces/IValidatorSigningAndQuery.sol";
import {ITeeKeyManager} from "src/interfaces/ITeeKeyManager.sol";
import {ProcessingStage, SequencerSignature} from "src/types/DataTypes.sol";
import {Ownable} from "@openzeppelin/contracts/access/Ownable.sol";

/**
 * @title ValidatorSignatureThresholdModule
 * @notice Validation module that requires a minimum number of validator signatures before message execution
 * @dev Enforces a configurable threshold of validator signatures during pre-execution validation.
 *      Validators are sourced from the TeeKeyManager contract (TEE-attested keys).
 */
contract ValidatorSignatureThresholdModule is IModuleCheck, Ownable {
    IValidatorSigningAndQuery public immutable BRIDGE;
    ITeeKeyManager public immutable TEE_KEY_MANAGER;
    uint256 public signatureThreshold;

    error ZeroAddress();
    error ThresholdZero();
    error ThresholdExceedsValidatorCount(uint256 threshold, uint256 validatorCount);

    /**
     * @notice Initializes the module with bridge, TEE key manager, and signature threshold
     * @param bridge Address of the bridge contract implementing IValidatorSigningAndQuery
     * @param teeKeyManager Address of the TEE key manager containing registered validators
     * @param _signatureThreshold Minimum number of signatures required (must be > 0)
     */
    constructor(address bridge, address teeKeyManager, uint256 _signatureThreshold) Ownable(msg.sender) {
        if (bridge == address(0) || teeKeyManager == address(0)) {
            revert ZeroAddress();
        }
        if (_signatureThreshold == 0) {
            revert ThresholdZero();
        }

        BRIDGE = IValidatorSigningAndQuery(bridge);
        TEE_KEY_MANAGER = ITeeKeyManager(teeKeyManager);
        signatureThreshold = _signatureThreshold;
    }

    /**
     * @notice Updates the signature threshold requirement
     * @dev Only callable by the contract owner
     * @param _signatureThreshold New minimum number of signatures required (must be > 0)
     */
    function setThreshold(uint256 _signatureThreshold) external onlyOwner {
        if (_signatureThreshold == 0) {
            revert ThresholdZero();
        }

        signatureThreshold = _signatureThreshold;
    }

    /**
     * @notice Validates that sufficient validator signatures exist for a message
     * @dev Only validates during PreExecution stage, checking signature count meets threshold.
     *      Validators are dynamically queried from TeeKeyManager.
     * @param messageId Unique identifier of the message being validated
     * @param stage Current processing stage (only validates during PreExecution)
     * @return bool True if signature threshold is met or stage is not PreExecution, false otherwise
     */
    function check(bytes32 messageId, ProcessingStage stage, bytes memory, SequencerSignature memory)
        external
        view
        returns (bool)
    {
        if (stage == ProcessingStage.PreExecution) {
            // Get current validators from TeeKeyManager
            address[] memory validators = TEE_KEY_MANAGER.getValidatorKeys();

            // If no validators registered, fail the check
            if (validators.length == 0) {
                return false;
            }

            uint256 signatureCount = BRIDGE.getValidatorSignatureCount(messageId, validators);

            if (signatureCount < signatureThreshold) {
                return false;
            }

            return true;
        }

        return true;
    }

    /**
     * @notice Gets the current validator count from TeeKeyManager
     * @return Number of registered validator keys
     */
    function getValidatorCount() external view returns (uint256) {
        return TEE_KEY_MANAGER.validatorKeyCount();
    }
}
