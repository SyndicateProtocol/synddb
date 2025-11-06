// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {IModuleCheck} from "src/interfaces/IModuleCheck.sol";
import {ProcessingStage, ValidatorSignatures} from "src/types/DataTypes.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";

contract ERC20SupplyCheckModule is IModuleCheck {
    IERC20 public immutable TOKEN;
    uint256 public immutable MAX_SUPPLY;

    constructor(address _token, uint256 _maxSupply) {
        TOKEN = IERC20(_token);
        MAX_SUPPLY = _maxSupply;
    }

    function check(ProcessingStage, bytes memory, ValidatorSignatures memory) external view returns (bool) {
        uint256 currentSupply = TOKEN.totalSupply();
        return currentSupply <= MAX_SUPPLY;
    }
}
