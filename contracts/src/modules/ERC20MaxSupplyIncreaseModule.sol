// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {IModuleCheck} from "src/interfaces/IModuleCheck.sol";
import {ProcessingStage, SequencerSignature} from "src/types/DataTypes.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";

contract ERC20MaxSupplyIncreaseModule is IModuleCheck {
    IERC20 public immutable TOKEN;
    uint256 public immutable MAX_SUPPLY_INCREASE;

    mapping(bytes32 messageId => uint256 supply) public preExecutionSupply;

    constructor(address _token, uint256 _maxSupplyIncrease) {
        TOKEN = IERC20(_token);
        MAX_SUPPLY_INCREASE = _maxSupplyIncrease;
    }

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
