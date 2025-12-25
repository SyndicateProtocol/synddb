// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

/**
 * @title MockTarget
 * @notice Mock contract for testing message execution
 */
contract MockTarget {
    uint256 public value;
    address public lastCaller;
    uint256 public lastValue;

    event ValueSet(uint256 indexed newValue, address indexed caller, uint256 ethValue);

    function setValue(uint256 _value) external payable {
        value = _value;
        lastCaller = msg.sender;
        lastValue = msg.value;
        emit ValueSet(_value, msg.sender, msg.value);
    }

    function revertingFunction() external pure {
        revert("MockTarget: reverted");
    }

    receive() external payable {}
}
