// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

/**
 * @title IBridge
 * @notice Interface for the SyndDB Bridge contract
 */
interface IBridge {
    // ============ Structs ============

    struct Validator {
        address publicKey;
        bytes32 attestationHash;
        bool isActive;
        uint256 addedAt;
    }

    struct DepositRecord {
        address depositor;
        address token;
        uint256 amount;
        uint256 blockNumber;
        bytes32 syndDbAccountId;
    }

    struct BalanceUpdate {
        address account;
        address token;
        int256 delta;
    }

    // ============ Events ============

    event Deposit(
        address indexed depositor,
        address indexed token,
        uint256 amount,
        bytes32 indexed syndDbAccountId,
        uint256 depositId
    );

    event Withdrawal(
        address indexed recipient,
        address indexed token,
        uint256 amount,
        uint256 nonce
    );

    event BatchSettlement(
        bytes32 indexed stateRoot,
        uint256 updateCount,
        uint256 nonce
    );

    event ValidatorAdded(address indexed validator, bytes32 attestationHash);
    event ValidatorRemoved(address indexed validator, string reason);
    event ValidatorUpdated(address indexed validator, bytes32 newAttestationHash);

    event CircuitBreakerTriggered(string reason, uint256 duration);
    event SequencerUpdated(address indexed oldSequencer, address indexed newSequencer);
    event RequiredSignaturesUpdated(uint256 oldValue, uint256 newValue);

    // ============ Deposit Functions ============

    function deposit(
        address token,
        uint256 amount,
        bytes32 syndDbAccountId
    ) external;

    function depositETH(bytes32 syndDbAccountId) external payable;

    // ============ Withdrawal Functions ============

    function processWithdrawal(
        uint256 nonce,
        address recipient,
        address token,
        uint256 amount,
        uint256 deadline,
        bytes memory sequencerSignature,
        bytes[] memory validatorSignatures
    ) external;

    function processBatchSettlement(
        uint256 nonce,
        bytes32 stateRoot,
        BalanceUpdate[] memory updates,
        uint256 deadline,
        bytes memory sequencerSignature,
        bytes[] memory validatorSignatures
    ) external;

    // ============ View Functions ============

    function getValidatorCount() external view returns (uint256);
    function getValidators() external view returns (address[] memory);
    function isValidator(address account) external view returns (bool);
    function getUserDepositBalance(address user, address token) external view returns (uint256);
    function getAccumulatedFees(address token) external view returns (uint256);

    function sequencer() external view returns (address);
    function requiredSignatures() external view returns (uint256);
    function processedNonces(uint256 nonce) external view returns (bool);
    function depositsEnabled() external view returns (bool);
    function withdrawalsEnabled() external view returns (bool);
}