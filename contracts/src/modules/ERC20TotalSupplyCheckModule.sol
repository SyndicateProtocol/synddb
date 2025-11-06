// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {IModuleCheck} from "src/interfaces/IModuleCheck.sol";
import {ProcessingStage, SequencerSignature} from "src/types/DataTypes.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";

contract ERC20TotalSupplyCheckModule is IModuleCheck {
    IERC20 public immutable TOKEN;
    uint256 public immutable TOTAL_SUPPLY_ALLOWED;

    constructor(address _token, uint256 _totalSupplyAllowed) {
        TOKEN = IERC20(_token);
        TOTAL_SUPPLY_ALLOWED = _totalSupplyAllowed;
    }

    function check(bytes32, ProcessingStage, bytes memory, SequencerSignature memory) external view returns (bool) {
        uint256 currentSupply = TOKEN.totalSupply();
        return currentSupply <= TOTAL_SUPPLY_ALLOWED;
    }
}
