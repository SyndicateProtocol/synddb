// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {ApplicationConfig, ValidatorInfo} from "src/types/DataTypes.sol";

/**
 * @title IApplicationRegistry
 * @notice Interface for managing application domain registration and validator configuration
 * @dev Each application has a domain identifier, Primary Validator, and configuration
 */
interface IApplicationRegistry {
    // ============================================================
    // EVENTS - Application Management
    // ============================================================

    /// @notice Emitted when a new application is registered
    event ApplicationRegistered(
        bytes32 indexed domain,
        address indexed primaryValidator,
        uint64 expirationSeconds,
        bool requireWitnessSignatures
    );

    /// @notice Emitted when an application's configuration is updated
    event ApplicationConfigUpdated(bytes32 indexed domain, address indexed primaryValidator);

    /// @notice Emitted when an application is activated or deactivated
    event ApplicationActiveChanged(bytes32 indexed domain, bool active);

    // ============================================================
    // EVENTS - Validator Management
    // ============================================================

    /// @notice Emitted when a Primary Validator is set for a domain
    event PrimaryValidatorSet(bytes32 indexed domain, address indexed oldValidator, address indexed newValidator);

    /// @notice Emitted when a Witness Validator is added
    event WitnessValidatorAdded(address indexed validator, bytes32 teeCodeHash, string teeVersion);

    /// @notice Emitted when a validator is removed
    event ValidatorRemoved(address indexed validator);

    /// @notice Emitted when the signature threshold is updated
    event ThresholdUpdated(uint256 oldThreshold, uint256 newThreshold);

    /// @notice Emitted when the minimum TEE version is updated
    event MinimumTeeVersionUpdated(string oldVersion, string newVersion);

    /// @notice Emitted when a validator's attestation is updated
    event ValidatorAttestationUpdated(address indexed validator, bytes32 teeCodeHash, string teeVersion);

    // ============================================================
    // APPLICATION MANAGEMENT
    // ============================================================

    /**
     * @notice Register a new application domain
     * @dev Only callable by REGISTRY_ROLE
     * @param domain Unique application identifier (typically keccak256 of app name)
     * @param config Application configuration including Primary Validator
     */
    function registerApplication(bytes32 domain, ApplicationConfig calldata config) external;

    /**
     * @notice Update an existing application's configuration
     * @dev Only callable by REGISTRY_ROLE
     * @param domain The application domain to update
     * @param config New application configuration
     */
    function updateApplicationConfig(bytes32 domain, ApplicationConfig calldata config) external;

    /**
     * @notice Activate or deactivate an application
     * @dev Only callable by REGISTRY_ROLE. Deactivated apps cannot initialize messages.
     * @param domain The application domain
     * @param active Whether to activate (true) or deactivate (false)
     */
    function setApplicationActive(bytes32 domain, bool active) external;

    // ============================================================
    // VALIDATOR MANAGEMENT
    // ============================================================

    /**
     * @notice Set the Primary Validator for a domain
     * @dev Only callable by VALIDATOR_MANAGER_ROLE
     * @param domain The application domain
     * @param validator The new Primary Validator address
     * @param attestation TEE attestation data for the validator
     */
    function setPrimaryValidator(bytes32 domain, address validator, bytes calldata attestation) external;

    /**
     * @notice Add a Witness Validator to the signing set
     * @dev Only callable by VALIDATOR_MANAGER_ROLE
     * @param validator The validator address to add
     * @param attestation TEE attestation data for the validator
     */
    function addWitnessValidator(address validator, bytes calldata attestation) external;

    /**
     * @notice Remove a validator from the signing set
     * @dev Only callable by VALIDATOR_MANAGER_ROLE
     * @param validator The validator address to remove
     */
    function removeValidator(address validator) external;

    /**
     * @notice Update a validator's TEE attestation
     * @dev Can be called to refresh attestation when TEE version changes
     * @param validator The validator to update
     * @param attestation New TEE attestation data
     * @param version New TEE version string
     */
    function updateValidatorAttestation(address validator, bytes calldata attestation, string calldata version) external;

    /**
     * @notice Set the signature threshold for message execution
     * @dev Only callable by ADMIN_ROLE
     * @param threshold New minimum number of signatures required
     */
    function setSignatureThreshold(uint256 threshold) external;

    /**
     * @notice Set the minimum required TEE version
     * @dev Only callable by ADMIN_ROLE
     * @param version Semantic version string (e.g., "1.2.3")
     */
    function setMinimumTeeVersion(string calldata version) external;

    // ============================================================
    // QUERY FUNCTIONS - Applications
    // ============================================================

    /**
     * @notice Get the configuration for an application domain
     * @param domain The application domain to query
     * @return config The application configuration
     */
    function getApplicationConfig(bytes32 domain) external view returns (ApplicationConfig memory config);

    /**
     * @notice Check if an application domain is registered
     * @param domain The domain to check
     * @return registered True if the domain exists
     */
    function isApplicationRegistered(bytes32 domain) external view returns (bool registered);

    /**
     * @notice Check if an application is active
     * @param domain The domain to check
     * @return active True if the application is active
     */
    function isApplicationActive(bytes32 domain) external view returns (bool active);

    /**
     * @notice Get the last consumed nonce for a domain
     * @param domain The application domain
     * @return nonce The last used nonce (next message should use nonce + 1)
     */
    function getLastNonce(bytes32 domain) external view returns (uint64 nonce);

    // ============================================================
    // QUERY FUNCTIONS - Validators
    // ============================================================

    /**
     * @notice Get information about a validator
     * @param validator The validator address to query
     * @return info The validator information
     */
    function getValidatorInfo(address validator) external view returns (ValidatorInfo memory info);

    /**
     * @notice Check if an address is an active validator
     * @param validator The address to check
     * @return active True if the address is an active validator
     */
    function isActiveValidator(address validator) external view returns (bool active);

    /**
     * @notice Check if an address is a Witness Validator
     * @param validator The address to check
     * @return isWitness True if the address is a Witness Validator
     */
    function isWitnessValidator(address validator) external view returns (bool isWitness);

    /**
     * @notice Get all active Witness Validators
     * @return validators Array of Witness Validator addresses
     */
    function getWitnessValidators() external view returns (address[] memory validators);

    /**
     * @notice Get the total number of active validators
     * @return count The number of active validators
     */
    function getActiveValidatorCount() external view returns (uint256 count);

    /**
     * @notice Get the current signature threshold
     * @return threshold The minimum number of signatures required
     */
    function getSignatureThreshold() external view returns (uint256 threshold);

    /**
     * @notice Get the minimum required TEE version
     * @return version The minimum TEE version string
     */
    function getMinimumTeeVersion() external view returns (string memory version);
}
