// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {IModuleCheck} from "src/interfaces/IModuleCheck.sol";
import {ProcessingStage, SequencerSignature} from "src/types/DataTypes.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";

/**
 * @title ERC20TotalSupplyCheckModule
 * @notice Validation module that enforces an absolute maximum total supply for an ERC20 token
 * @dev Validates that the token's total supply never exceeds the configured limit
 */
contract ERC20TotalSupplyCheckModule is IModuleCheck {
    IERC20 public immutable TOKEN;
    uint256 public immutable TOTAL_SUPPLY_ALLOWED;

    /**
     * @notice Initializes the module with token and maximum total supply
     * @param _token Address of the ERC20 token to monitor
     * @param _totalSupplyAllowed Maximum allowed total supply for the token
     */
    constructor(address _token, uint256 _totalSupplyAllowed) {
        TOKEN = IERC20(_token);
        TOTAL_SUPPLY_ALLOWED = _totalSupplyAllowed;
    }

    /**
     * @notice Validates that the token's total supply doesn't exceed the maximum
     * @dev Can be called at any processing stage, always checks current supply against limit
     * @return bool True if current supply is within the allowed limit, false otherwise
     */
    function check(bytes32, ProcessingStage, bytes memory, SequencerSignature memory) external view returns (bool) {
        uint256 currentSupply = TOKEN.totalSupply();
        return currentSupply <= TOTAL_SUPPLY_ALLOWED;
    }
}
