// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {MessageTypeConfig} from "src/types/DataTypes.sol";

/**
 * @title IMessageTypeRegistry
 * @notice Interface for managing message type registration and configuration
 * @dev Message types define the schema and target for each supported message format
 */
interface IMessageTypeRegistry {
    // ============================================================
    // EVENTS
    // ============================================================

    /// @notice Emitted when a new message type is registered
    event MessageTypeRegistered(
        string indexed messageType, bytes4 selector, address target, bytes32 schemaHash, string schemaUri
    );

    /// @notice Emitted when a message type's schema is updated
    event SchemaUpdated(string indexed messageType, bytes32 oldSchemaHash, bytes32 newSchemaHash, string newSchemaUri);

    /// @notice Emitted when a message type is enabled or disabled
    event MessageTypeEnabled(string indexed messageType, bool enabled);

    /// @notice Emitted when a message type's target is updated
    event TargetUpdated(string indexed messageType, address oldTarget, address newTarget);

    // ============================================================
    // REGISTRATION FUNCTIONS
    // ============================================================

    /**
     * @notice Register a new message type
     * @dev Only callable by REGISTRY_ROLE
     * @param messageType The ABI function signature (e.g., "mint(address,uint256)")
     * @param target The contract address to call when executing this message type
     * @param schemaHash keccak256 of the metadata JSON Schema
     * @param schemaUri URI where the full schema can be fetched (IPFS, Arweave)
     */
    function registerMessageType(
        string calldata messageType,
        address target,
        bytes32 schemaHash,
        string calldata schemaUri
    ) external;

    /**
     * @notice Update the schema for an existing message type
     * @dev Only callable by REGISTRY_ROLE
     * @param messageType The message type to update
     * @param newSchemaHash New keccak256 of the metadata JSON Schema
     * @param newSchemaUri New URI where the full schema can be fetched
     */
    function updateSchema(string calldata messageType, bytes32 newSchemaHash, string calldata newSchemaUri) external;

    /**
     * @notice Enable or disable a message type
     * @dev Only callable by REGISTRY_ROLE. Disabled types cannot be initialized.
     * @param messageType The message type to enable/disable
     * @param enabled Whether to enable (true) or disable (false)
     */
    function setEnabled(string calldata messageType, bool enabled) external;

    /**
     * @notice Update the target contract for a message type
     * @dev Only callable by REGISTRY_ROLE
     * @param messageType The message type to update
     * @param newTarget New target contract address
     */
    function setTarget(string calldata messageType, address newTarget) external;

    // ============================================================
    // QUERY FUNCTIONS
    // ============================================================

    /**
     * @notice Get the full configuration for a message type
     * @param messageType The message type to query
     * @return config The message type configuration
     */
    function getMessageTypeConfig(string calldata messageType) external view returns (MessageTypeConfig memory config);

    /**
     * @notice Check if a message type is registered
     * @param messageType The message type to check
     * @return registered True if the message type exists
     */
    function isMessageTypeRegistered(string calldata messageType) external view returns (bool registered);

    /**
     * @notice Check if a message type is enabled
     * @param messageType The message type to check
     * @return enabled True if the message type is enabled
     */
    function isMessageTypeEnabled(string calldata messageType) external view returns (bool enabled);

    /**
     * @notice Get the message type string for a function selector
     * @dev Useful for reverse lookup from calldata
     * @param selector The function selector (first 4 bytes of keccak256)
     * @return messageType The corresponding message type string
     */
    function getMessageTypeBySelector(bytes4 selector) external view returns (string memory messageType);

    /**
     * @notice Get the target contract for a message type
     * @param messageType The message type to query
     * @return target The target contract address
     */
    function getTarget(string calldata messageType) external view returns (address target);

    /**
     * @notice Get the schema hash for a message type
     * @param messageType The message type to query
     * @return schemaHash The keccak256 hash of the JSON Schema
     */
    function getSchemaHash(string calldata messageType) external view returns (bytes32 schemaHash);
}
