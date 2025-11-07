// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

interface IModuleCheckRegistry {
    function addPreModule(address module) external;
    function addPostModule(address module) external;
    function removePreModule(address module) external;
    function removePostModule(address module) external;
    function getPreModules() external view returns (address[] memory);
    function getPostModules() external view returns (address[] memory);
}
