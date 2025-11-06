// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {IModuleChecker} from "../interfaces/IModuleChecker.sol";
import {IBridge} from "../interfaces/IBridge.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";

contract ERC20SupplyCheckModule is IModuleChecker {
    IERC20 public immutable TOKEN;
    uint256 public immutable MAX_SUPPLY;

    constructor(address _token, uint256 _maxSupply) {
        TOKEN = IERC20(_token);
        MAX_SUPPLY = _maxSupply;
    }

    function check(IBridge.ProcessingStage, bytes memory, IBridge.ValidatorSignatures memory)
        external
        view
        returns (bool)
    {
        uint256 currentSupply = TOKEN.totalSupply();
        return currentSupply <= MAX_SUPPLY;
    }
}
