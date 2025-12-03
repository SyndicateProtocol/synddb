// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

/**
 * @title IModuleCheckRegistry
 * @notice Interface for managing validation modules that run before and after message execution
 * @dev Pre-modules validate conditions before execution, post-modules validate state changes after execution
 */
interface IModuleCheckRegistry {
    /**
     * @notice Adds a validation module to run before message execution
     * @param module Address of the module implementing IModuleCheck
     */
    function addPreModule(address module) external;

    /**
     * @notice Adds a validation module to run after message execution
     * @param module Address of the module implementing IModuleCheck
     */
    function addPostModule(address module) external;

    /**
     * @notice Removes a pre-execution validation module
     * @param module Address of the module to remove
     */
    function removePreModule(address module) external;

    /**
     * @notice Removes a post-execution validation module
     * @param module Address of the module to remove
     */
    function removePostModule(address module) external;

    /**
     * @notice Gets all registered pre-execution modules
     * @dev Returns `address[] memory` instead of `address[] calldata` because the data is constructed
     *      from storage (EnumerableSet) and returned, not passed in as calldata.
     * @return Array of module addresses that run before execution
     */
    function getPreModules() external view returns (address[] memory);

    /**
     * @notice Gets all registered post-execution modules
     * @dev Returns `address[] memory` instead of `address[] calldata` because the data is constructed
     *      from storage (EnumerableSet) and returned, not passed in as calldata.
     * @return Array of module addresses that run after execution
     */
    function getPostModules() external view returns (address[] memory);
}
