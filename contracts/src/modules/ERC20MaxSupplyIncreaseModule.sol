// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {IModuleCheck} from "src/interfaces/IModuleCheck.sol";
import {ProcessingStage, SequencerSignature} from "src/types/DataTypes.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";

/**
 * @title ERC20MaxSupplyIncreaseModule
 * @notice Validation module that limits how much an ERC20 token's supply can increase per message
 * @dev Captures supply at pre-execution and validates the increase doesn't exceed the maximum at post-execution
 */
contract ERC20MaxSupplyIncreaseModule is IModuleCheck {
    IERC20 public immutable TOKEN;
    uint256 public immutable MAX_SUPPLY_INCREASE;

    mapping(bytes32 messageId => uint256 supply) public preExecutionSupply;

    /**
     * @notice Initializes the module with token and maximum supply increase
     * @param _token Address of the ERC20 token to monitor
     * @param _maxSupplyIncrease Maximum allowed supply increase per message
     */
    constructor(address _token, uint256 _maxSupplyIncrease) {
        TOKEN = IERC20(_token);
        MAX_SUPPLY_INCREASE = _maxSupplyIncrease;
    }

    /**
     * @notice Validates that token supply increase doesn't exceed the maximum
     * @dev Pre-execution: Records current supply. Post-execution: Validates increase is within limit
     * @param messageId Unique identifier of the message being validated
     * @param stage Current processing stage (PreExecution or PostExecution)
     * @return bool True if validation passes, false if supply increase exceeds maximum
     */
    function check(bytes32 messageId, ProcessingStage stage, bytes memory, SequencerSignature memory)
        external
        returns (bool)
    {
        uint256 currentSupply = TOKEN.totalSupply();

        if (stage == ProcessingStage.PreExecution) {
            preExecutionSupply[messageId] = currentSupply;
            return true;
        } else if (stage == ProcessingStage.PostExecution) {
            uint256 storedSupply = preExecutionSupply[messageId];
            if (currentSupply > storedSupply) {
                uint256 supplyIncrease = currentSupply - storedSupply;
                if (supplyIncrease > MAX_SUPPLY_INCREASE) {
                    return false;
                }
            }

            delete preExecutionSupply[messageId];
            return true;
        }

        return true;
    }
}
