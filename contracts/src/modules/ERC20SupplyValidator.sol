// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {IModuleValidator} from "../interfaces/IModuleValidator.sol";
import {IBridge} from "../interfaces/IBridge.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";

contract ERC20SupplyValidator is IModuleValidator {
    IERC20 public immutable TOKEN;
    uint256 public immutable MAX_SUPPLY;

    constructor(address _token, uint256 _maxSupply) {
        TOKEN = IERC20(_token);
        MAX_SUPPLY = _maxSupply;
    }

    function validate(IBridge.ProcessingStage, bytes calldata) external view returns (bool) {
        uint256 currentSupply = TOKEN.totalSupply();
        return currentSupply <= MAX_SUPPLY;
    }
}
