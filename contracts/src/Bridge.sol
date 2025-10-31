// SPDX-License-Identifier: MIT
pragma solidity ^0.8.30;

import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {SafeERC20} from "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";
import {ReentrancyGuard} from "@openzeppelin/contracts/utils/ReentrancyGuard.sol";
import {Pausable} from "@openzeppelin/contracts/utils/Pausable.sol";
import {Ownable} from "@openzeppelin/contracts/access/Ownable.sol";
import {ECDSA} from "@openzeppelin/contracts/utils/cryptography/ECDSA.sol";
import {EIP712} from "@openzeppelin/contracts/utils/cryptography/EIP712.sol";
import {Address} from "@openzeppelin/contracts/utils/Address.sol";

/**
 * @title SyndDBBridge
 * @notice Bridge contract for SyndDB enabling deposits, withdrawals, and batch settlements
 * @dev Supports complex orderbook operations with m-of-n validator signatures
 */
contract SyndDBBridge is ReentrancyGuard, Pausable, Ownable, EIP712 {
    using SafeERC20 for IERC20;
    using ECDSA for bytes32;
    using Address for address payable;

    // ============ Constants ============

    // EIP-712 type hashes
    bytes32 public constant WITHDRAWAL_TYPEHASH =
        keccak256("Withdrawal(uint256 nonce,address recipient,address token,uint256 amount,uint256 deadline)");
    bytes32 public constant BATCH_SETTLEMENT_TYPEHASH =
        keccak256("BatchSettlement(uint256 nonce,bytes32 stateRoot,uint256 deadline)");

    // ============ State Variables ============

    // Validator management
    struct Validator {
        address publicKey;
        bytes32 attestationHash;
        bool isActive;
        uint256 addedAt;
        bytes32 initialWasmVersionHash; // Initial WASM version at registration
    }

    mapping(address => Validator) public validators;
    mapping(uint256 => address) public validatorByIndex;
    uint256 public validatorCount;
    uint256 public requiredSignatures; // m in m-of-n
    address public sequencer;

    // TEE Relayer management
    struct Relayer {
        address publicKey;
        bytes32 attestationHash;
        bool isActive;
        uint256 addedAt;
        bytes32 teeVersionHash;
    }

    mapping(address => Relayer) public relayers;
    mapping(uint256 => address) public relayerByIndex;
    uint256 public relayerCount;

    // Deposit tracking - rely primarily on events for deposit history
    uint256 public totalDeposits;

    // Withdrawal tracking
    mapping(uint256 => bool) public processedNonces;
    uint256 public currentNonce;

    // Balance updates for batch settlements
    enum BalanceUpdateType {
        Credit, // Withdrawal from SyndDB to user
        Debit // Should have been pre-deposited
    }

    struct BalanceUpdate {
        address account;
        address token;
        uint256 amount;
        BalanceUpdateType updateType;
    }

    // Circuit breakers - global limits across all tokens
    struct WithdrawalWindow {
        uint256 amount;
        uint256 windowStart;
    }

    mapping(address => WithdrawalWindow) public perTokenDailyWithdrawal; // token => withdrawal tracking
    mapping(address => mapping(address => WithdrawalWindow)) public perUserPerTokenDailyWithdrawal; // user => token => withdrawal tracking

    uint256 public globalDailyLimitPerToken; // Max withdrawal per token per day
    uint256 public perUserDailyLimitPerToken; // Max withdrawal per user per token per day
    uint256 public constant WITHDRAWAL_WINDOW_DURATION = 1 days;

    // Emergency controls
    bool public depositsEnabled = true;
    bool public withdrawalsEnabled = true;
    uint256 public emergencyPauseDeadline; // Minimum time before unpause is allowed (automatic unpause)

    // ============ Events ============
    event Deposit(
        address indexed depositor,
        address indexed token,
        uint256 amount,
        bytes32 indexed syndDbAccountId,
        uint256 depositId
    );

    event Withdrawal(address indexed recipient, address indexed token, uint256 amount, uint256 nonce);

    event BatchSettlement(bytes32 indexed stateRoot, uint256 updateCount, uint256 nonce);

    event ValidatorAdded(address indexed validator, bytes32 attestationHash);
    event ValidatorRemoved(address indexed validator, string reason);
    event ValidatorUpdated(address indexed validator, bytes32 newAttestationHash);

    event RelayerAdded(address indexed relayer, bytes32 attestationHash);
    event RelayerRemoved(address indexed relayer, string reason);
    event RelayerUpdated(address indexed relayer, bytes32 newAttestationHash);

    event CircuitBreakerTriggered(string reason, uint256 duration);
    event SequencerUpdated(address indexed oldSequencer, address indexed newSequencer);
    event RequiredSignaturesUpdated(uint256 oldValue, uint256 newValue);

    event EmergencyPause(uint256 deadline);

    // ============ Modifiers ============
    modifier onlySequencer() {
        require(isSequencer(msg.sender), "Not sequencer");
        _;
    }

    modifier onlyRelayer() {
        require(isActiveRelayer(msg.sender), "Not active relayer");
        _;
    }

    modifier whenDepositsEnabled() {
        require(areDepositsEnabled(), "Deposits disabled");
        _;
    }

    modifier whenWithdrawalsEnabled() {
        require(areWithdrawalsEnabled(), "Withdrawals disabled");
        _;
    }

    // Public view functions for checking status
    function isSequencer(address account) public view returns (bool) {
        return account == sequencer;
    }

    function isActiveRelayer(address account) public view returns (bool) {
        return relayers[account].isActive;
    }

    function areDepositsEnabled() public view returns (bool) {
        return depositsEnabled;
    }

    function areWithdrawalsEnabled() public view returns (bool) {
        return withdrawalsEnabled;
    }

    // ============ Constructor ============
    /**
     * @notice Initializes the SyndDB Bridge contract
     * @param _sequencer The address of the sequencer responsible for ordering transactions
     * @param _initialValidators Array of initial validator addresses to bootstrap the bridge
     * @param _requiredSignatures The number of validator signatures required for withdrawals (m in m-of-n multisig)
     * @param _globalDailyLimitPerToken Maximum amount that can be withdrawn per token per day
     * @param _perUserDailyLimitPerToken Maximum amount that can be withdrawn per user per token per day
     */
    constructor(
        address _sequencer,
        address[] memory _initialValidators,
        uint256 _requiredSignatures,
        uint256 _globalDailyLimitPerToken,
        uint256 _perUserDailyLimitPerToken
    ) Ownable(msg.sender) EIP712("SyndDBBridge", "1") {
        require(_sequencer != address(0), "Invalid sequencer");
        require(_initialValidators.length > 0, "Need at least one validator");
        require(_requiredSignatures <= _initialValidators.length, "Invalid signature requirement");
        require(_requiredSignatures >= 1, "At least 1 signature required");

        sequencer = _sequencer;
        requiredSignatures = _requiredSignatures;
        globalDailyLimitPerToken = _globalDailyLimitPerToken;
        perUserDailyLimitPerToken = _perUserDailyLimitPerToken;

        for (uint256 i = 0; i < _initialValidators.length; i++) {
            require(_initialValidators[i] != address(0), "Invalid validator");
            validators[_initialValidators[i]] = Validator({
                publicKey: _initialValidators[i],
                attestationHash: bytes32(0),
                isActive: true,
                addedAt: block.timestamp,
                initialWasmVersionHash: bytes32(0)
            });
            validatorByIndex[i] = _initialValidators[i];
        }
        validatorCount = _initialValidators.length;
    }

    // ============ Deposit Functions ============

    /**
     * @notice Deposit ERC20 tokens to the bridge
     * @param token The ERC20 token address
     * @param amount The amount to deposit
     * @param syndDbAccountId The account ID in SyndDB to credit
     */
    function deposit(address token, uint256 amount, bytes32 syndDbAccountId)
        external
        nonReentrant
        whenNotPaused
        whenDepositsEnabled
    {
        require(token != address(0), "Invalid token");
        require(amount > 0, "Zero amount");
        require(syndDbAccountId != bytes32(0), "Invalid SyndDB account");

        // Transfer tokens from user
        IERC20(token).safeTransferFrom(msg.sender, address(this), amount);

        // Increment deposit counter
        uint256 depositId = totalDeposits++;

        emit Deposit(msg.sender, token, amount, syndDbAccountId, depositId);
    }

    /**
     * @notice Deposit ETH to the bridge
     * @param syndDbAccountId The account ID in SyndDB to credit
     */
    function depositEth(bytes32 syndDbAccountId) external payable nonReentrant whenNotPaused whenDepositsEnabled {
        require(msg.value > 0, "Zero amount");
        require(syndDbAccountId != bytes32(0), "Invalid SyndDB account");

        // Increment deposit counter
        uint256 depositId = totalDeposits++;

        emit Deposit(msg.sender, address(0), msg.value, syndDbAccountId, depositId);
    }

    // ============ Withdrawal Functions ============

    /**
     * @notice Process a withdrawal with validator signatures (only callable by TEE relayer)
     * @param nonce The unique nonce for this withdrawal
     * @param recipient The recipient address
     * @param token The token to withdraw
     * @param amount The amount to withdraw
     * @param deadline The deadline for this withdrawal
     * @param sequencerSignature The sequencer's signature
     * @param validatorSignatures Array of validator signatures
     */
    function processWithdrawal(
        uint256 nonce,
        address recipient,
        address token,
        uint256 amount,
        uint256 deadline,
        bytes memory sequencerSignature,
        bytes[] memory validatorSignatures
    ) external onlyRelayer nonReentrant whenNotPaused whenWithdrawalsEnabled {
        // Check deadline
        require(block.timestamp <= deadline, "Withdrawal expired");
        require(!processedNonces[nonce], "Nonce already processed");
        require(validatorSignatures.length >= requiredSignatures, "Insufficient signatures");

        // Check circuit breakers
        require(checkWithdrawalLimits(token, amount, recipient), "Withdrawal limit exceeded");

        // Construct and verify the EIP-712 withdrawal message
        // The sequencer and validators sign the withdrawal parameters (nonce, recipient, token, amount, deadline)
        // We reconstruct the message here to verify that their signatures are valid for these specific parameters
        bytes32 structHash = keccak256(abi.encode(WITHDRAWAL_TYPEHASH, nonce, recipient, token, amount, deadline));
        bytes32 messageHash = _hashTypedDataV4(structHash);

        // Verify sequencer signature
        address recoveredSequencer = messageHash.recover(sequencerSignature);
        require(recoveredSequencer == sequencer, "Invalid sequencer signature");

        // Verify validator signatures
        uint256 validSignatures = 0;
        address[] memory signers = new address[](validatorSignatures.length);

        for (uint256 i = 0; i < validatorSignatures.length; i++) {
            address signer = messageHash.recover(validatorSignatures[i]);

            // Check if signer is a valid validator
            if (validators[signer].isActive) {
                // Check for duplicate signers
                bool isDuplicate = false;
                for (uint256 j = 0; j < i; j++) {
                    if (signers[j] == signer) {
                        isDuplicate = true;
                        break;
                    }
                }

                if (!isDuplicate) {
                    signers[i] = signer;
                    validSignatures++;
                }
            }
        }

        require(validSignatures >= requiredSignatures, "Insufficient valid signatures");

        // Mark nonce as processed
        processedNonces[nonce] = true;

        // Update circuit breaker tracking
        updateWithdrawalTracking(token, amount, recipient);

        // Execute withdrawal
        if (token == address(0)) {
            // ETH withdrawal
            payable(recipient).sendValue(amount);
        } else {
            // ERC20 withdrawal
            IERC20(token).safeTransfer(recipient, amount);
        }

        emit Withdrawal(recipient, token, amount, nonce);
    }

    /**
     * @notice Process a batch settlement for various use cases (only callable by TEE relayer)
     * @dev The state root provides a commitment to the batch of updates for efficient verification
     *      and enables off-chain validation/data availability. Validators sign the state root
     *      rather than individual updates for gas efficiency.
     * @param nonce The unique nonce for this settlement
     * @param stateRoot The merkle root of balance updates (commitment for batch verification)
     * @param updates Array of balance updates to apply
     * @param deadline The deadline for this settlement
     * @param sequencerSignature The sequencer's signature
     * @param validatorSignatures Array of validator signatures
     */
    function processBatchSettlement(
        uint256 nonce,
        bytes32 stateRoot,
        BalanceUpdate[] memory updates,
        uint256 deadline,
        bytes memory sequencerSignature,
        bytes[] memory validatorSignatures
    ) external onlyRelayer nonReentrant whenNotPaused whenWithdrawalsEnabled {
        require(block.timestamp <= deadline, "Settlement expired");
        require(!processedNonces[nonce], "Nonce already processed");
        require(validatorSignatures.length >= requiredSignatures, "Insufficient signatures");
        require(updates.length > 0 && updates.length <= 100, "Invalid update count");

        // Verify state root matches updates
        bytes32 calculatedRoot = calculateMerkleRoot(updates);
        require(calculatedRoot == stateRoot, "State root mismatch");

        // Construct the settlement message - validators sign the state root for efficiency
        bytes32 structHash = keccak256(abi.encode(BATCH_SETTLEMENT_TYPEHASH, nonce, stateRoot, deadline));
        bytes32 messageHash = _hashTypedDataV4(structHash);

        // Verify sequencer signature
        address recoveredSequencer = messageHash.recover(sequencerSignature);
        require(recoveredSequencer == sequencer, "Invalid sequencer signature");

        // Verify validator signatures
        uint256 validSignatures = verifyValidatorSignatures(messageHash, validatorSignatures);
        require(validSignatures >= requiredSignatures, "Insufficient valid signatures");

        // Mark nonce as processed
        processedNonces[nonce] = true;

        // Process all balance updates atomically
        for (uint256 i = 0; i < updates.length; i++) {
            BalanceUpdate memory update = updates[i];

            if (update.updateType == BalanceUpdateType.Credit) {
                // Credit to user (withdrawal from SyndDB to L1)
                require(checkWithdrawalLimits(update.token, update.amount, update.account), "Withdrawal limit exceeded");
                updateWithdrawalTracking(update.token, update.amount, update.account);

                if (update.token == address(0)) {
                    payable(update.account).sendValue(update.amount);
                } else {
                    IERC20(update.token).safeTransfer(update.account, update.amount);
                }
            } else {
                // Debit type - currently not used, but reserved for future functionality
                // Note: Debits would require pre-approved deposit tracking
                revert("Debit operations not supported in batch settlements");
            }
        }

        emit BatchSettlement(stateRoot, updates.length, nonce);
    }

    // ============ Validator Management ============

    /**
     * @notice Add a new validator with TEE attestation
     * @param validator The validator address
     * @param attestationHash The hash of the TEE attestation
     * @param wasmVersionHash The hash of the initial WASM version (static, set at registration)
     */
    function addValidator(address validator, bytes32 attestationHash, bytes32 wasmVersionHash) external onlyOwner {
        require(validator != address(0), "Invalid validator");
        require(!validators[validator].isActive, "Already active");

        validators[validator] = Validator({
            publicKey: validator,
            attestationHash: attestationHash,
            isActive: true,
            addedAt: block.timestamp,
            initialWasmVersionHash: wasmVersionHash
        });

        validatorByIndex[validatorCount] = validator;
        validatorCount++;

        emit ValidatorAdded(validator, attestationHash);
    }

    /**
     * @notice Remove a validator
     * @param validatorIndex The index of the validator to remove
     * @param reason The reason for removal
     */
    function removeValidator(uint256 validatorIndex, string memory reason) external onlyOwner {
        require(validatorIndex < validatorCount, "Invalid index");
        address validator = validatorByIndex[validatorIndex];
        require(validators[validator].isActive, "Not active");
        require(validatorCount - 1 >= requiredSignatures, "Would break signature requirement");

        validators[validator].isActive = false;

        // Move last validator to the removed slot
        if (validatorIndex != validatorCount - 1) {
            validatorByIndex[validatorIndex] = validatorByIndex[validatorCount - 1];
        }
        delete validatorByIndex[validatorCount - 1];
        validatorCount--;

        emit ValidatorRemoved(validator, reason);
    }

    /**
     * @notice Update validator attestation when the TEE attestation needs to be refreshed
     * @dev This updates the attestation hash when the validator's TEE re-attests.
     *      The WASM version is immutable and set at registration time.
     * @param validator The validator address
     * @param newAttestationHash The new attestation hash
     */
    function updateValidatorAttestation(address validator, bytes32 newAttestationHash) external onlyOwner {
        require(validators[validator].isActive, "Not active");
        validators[validator].attestationHash = newAttestationHash;
        emit ValidatorUpdated(validator, newAttestationHash);
    }

    // ============ Relayer Management ============

    /**
     * @notice Add a new TEE relayer with attestation
     * @param relayer The relayer address
     * @param attestationHash The hash of the TEE attestation
     * @param teeVersionHash The hash of the TEE version
     */
    function addRelayer(address relayer, bytes32 attestationHash, bytes32 teeVersionHash) external onlyOwner {
        require(relayer != address(0), "Invalid relayer");
        require(!relayers[relayer].isActive, "Already active");

        relayers[relayer] = Relayer({
            publicKey: relayer,
            attestationHash: attestationHash,
            isActive: true,
            addedAt: block.timestamp,
            teeVersionHash: teeVersionHash
        });

        relayerByIndex[relayerCount] = relayer;
        relayerCount++;

        emit RelayerAdded(relayer, attestationHash);
    }

    /**
     * @notice Remove a relayer
     * @param relayerIndex The index of the relayer to remove
     * @param reason The reason for removal
     */
    function removeRelayer(uint256 relayerIndex, string memory reason) external onlyOwner {
        require(relayerIndex < relayerCount, "Invalid index");
        address relayer = relayerByIndex[relayerIndex];
        require(relayers[relayer].isActive, "Not active");
        require(relayerCount > 1, "Cannot remove last relayer");

        relayers[relayer].isActive = false;

        // Move last relayer to the removed slot
        if (relayerIndex != relayerCount - 1) {
            relayerByIndex[relayerIndex] = relayerByIndex[relayerCount - 1];
        }
        delete relayerByIndex[relayerCount - 1];
        relayerCount--;

        emit RelayerRemoved(relayer, reason);
    }

    /**
     * @notice Update relayer attestation and TEE version
     * @param relayer The relayer address
     * @param newAttestationHash The new attestation hash
     * @param newTeeVersionHash The new TEE version hash
     */
    function updateRelayerAttestation(address relayer, bytes32 newAttestationHash, bytes32 newTeeVersionHash)
        external
        onlyOwner
    {
        require(relayers[relayer].isActive, "Not active");
        relayers[relayer].attestationHash = newAttestationHash;
        relayers[relayer].teeVersionHash = newTeeVersionHash;
        emit RelayerUpdated(relayer, newAttestationHash);
    }

    // ============ Circuit Breaker Functions ============

    /**
     * @notice Check if withdrawal is within limits
     */
    function checkWithdrawalLimits(address token, uint256 amount, address user) internal view returns (bool) {
        // Check global per-token daily limit
        WithdrawalWindow memory tokenWindow = perTokenDailyWithdrawal[token];
        if (block.timestamp >= tokenWindow.windowStart + WITHDRAWAL_WINDOW_DURATION) {
            // Window has expired, amount would be the first in new window
            if (amount > globalDailyLimitPerToken) {
                return false;
            }
        } else {
            // Window is still active
            if (tokenWindow.amount + amount > globalDailyLimitPerToken) {
                return false;
            }
        }

        // Check per-user per-token daily limit
        WithdrawalWindow memory userWindow = perUserPerTokenDailyWithdrawal[user][token];
        if (block.timestamp >= userWindow.windowStart + WITHDRAWAL_WINDOW_DURATION) {
            // Window has expired, amount would be the first in new window
            if (amount > perUserDailyLimitPerToken) {
                return false;
            }
        } else {
            // Window is still active
            if (userWindow.amount + amount > perUserDailyLimitPerToken) {
                return false;
            }
        }

        return true;
    }

    /**
     * @notice Update withdrawal tracking for circuit breakers with automatic window rollover
     */
    function updateWithdrawalTracking(address token, uint256 amount, address user) internal {
        // Update global per-token tracking
        WithdrawalWindow storage tokenWindow = perTokenDailyWithdrawal[token];
        if (block.timestamp >= tokenWindow.windowStart + WITHDRAWAL_WINDOW_DURATION) {
            // Start new window
            tokenWindow.amount = amount;
            tokenWindow.windowStart = block.timestamp;
        } else {
            // Add to current window
            tokenWindow.amount += amount;
        }

        // Update per-user per-token tracking
        WithdrawalWindow storage userWindow = perUserPerTokenDailyWithdrawal[user][token];
        if (block.timestamp >= userWindow.windowStart + WITHDRAWAL_WINDOW_DURATION) {
            // Start new window
            userWindow.amount = amount;
            userWindow.windowStart = block.timestamp;
        } else {
            // Add to current window
            userWindow.amount += amount;
        }
    }

    // ============ Admin Functions ============

    /**
     * @notice Update the sequencer address
     */
    function updateSequencer(address newSequencer) external onlyOwner {
        require(newSequencer != address(0), "Invalid sequencer");
        address oldSequencer = sequencer;
        sequencer = newSequencer;
        emit SequencerUpdated(oldSequencer, newSequencer);
    }

    /**
     * @notice Update required signatures
     */
    function updateRequiredSignatures(uint256 newRequired) external onlyOwner {
        require(newRequired > 0 && newRequired <= validatorCount, "Invalid requirement");
        uint256 oldRequired = requiredSignatures;
        requiredSignatures = newRequired;
        emit RequiredSignaturesUpdated(oldRequired, newRequired);
    }

    /**
     * @notice Set withdrawal limits
     */
    function setWithdrawalLimits(uint256 _globalDailyLimitPerToken, uint256 _perUserDailyLimitPerToken)
        external
        onlyOwner
    {
        globalDailyLimitPerToken = _globalDailyLimitPerToken;
        perUserDailyLimitPerToken = _perUserDailyLimitPerToken;
    }

    /**
     * @notice Emergency pause
     */
    function emergencyPause(uint256 duration) external onlyOwner {
        _pause();
        emergencyPauseDeadline = block.timestamp + duration;
        emit EmergencyPause(emergencyPauseDeadline);
        emit CircuitBreakerTriggered("Emergency pause", duration);
    }

    /**
     * @notice Resume operations
     */
    function unpause() external onlyOwner {
        require(block.timestamp >= emergencyPauseDeadline, "Still in emergency pause period");
        _unpause();
    }

    /**
     * @notice Toggle deposits
     */
    function setDepositsEnabled(bool enabled) external onlyOwner {
        depositsEnabled = enabled;
    }

    /**
     * @notice Toggle withdrawals
     */
    function setWithdrawalsEnabled(bool enabled) external onlyOwner {
        withdrawalsEnabled = enabled;
    }

    // ============ Helper Functions ============

    /**
     * @notice Calculate merkle root for balance updates
     */
    function calculateMerkleRoot(BalanceUpdate[] memory updates) internal pure returns (bytes32) {
        if (updates.length == 0) return bytes32(0);

        bytes32[] memory leaves = new bytes32[](updates.length);
        for (uint256 i = 0; i < updates.length; i++) {
            leaves[i] = keccak256(abi.encode(updates[i]));
        }

        // Simple merkle tree construction (for demonstration)
        while (leaves.length > 1) {
            bytes32[] memory newLevel = new bytes32[]((leaves.length + 1) / 2);
            for (uint256 i = 0; i < leaves.length; i += 2) {
                if (i + 1 < leaves.length) {
                    newLevel[i / 2] = keccak256(abi.encodePacked(leaves[i], leaves[i + 1]));
                } else {
                    newLevel[i / 2] = leaves[i];
                }
            }
            leaves = newLevel;
        }

        return leaves[0];
    }

    /**
     * @notice Verify validator signatures
     */
    function verifyValidatorSignatures(bytes32 messageHash, bytes[] memory signatures) internal view returns (uint256) {
        uint256 validSignatures = 0;
        address[] memory signers = new address[](signatures.length);

        for (uint256 i = 0; i < signatures.length; i++) {
            address signer = messageHash.recover(signatures[i]);

            if (validators[signer].isActive) {
                bool isDuplicate = false;
                for (uint256 j = 0; j < i; j++) {
                    if (signers[j] == signer) {
                        isDuplicate = true;
                        break;
                    }
                }

                if (!isDuplicate) {
                    signers[i] = signer;
                    validSignatures++;
                }
            }
        }

        return validSignatures;
    }

    // ============ View Functions ============

    /**
     * @notice Get validator count
     */
    function getValidatorCount() external view returns (uint256) {
        return validatorCount;
    }

    /**
     * @notice Get all validators
     */
    function getValidators() external view returns (address[] memory) {
        address[] memory validatorAddresses = new address[](validatorCount);
        for (uint256 i = 0; i < validatorCount; i++) {
            validatorAddresses[i] = validatorByIndex[i];
        }
        return validatorAddresses;
    }

    /**
     * @notice Check if an address is an active validator
     */
    function isValidator(address account) external view returns (bool) {
        return validators[account].isActive;
    }

    /**
     * @notice Get relayer count
     */
    function getRelayerCount() external view returns (uint256) {
        return relayerCount;
    }

    /**
     * @notice Get all relayers
     */
    function getRelayers() external view returns (address[] memory) {
        address[] memory relayerAddresses = new address[](relayerCount);
        for (uint256 i = 0; i < relayerCount; i++) {
            relayerAddresses[i] = relayerByIndex[i];
        }
        return relayerAddresses;
    }

    /**
     * @notice Check if an address is an active relayer
     */
    function isRelayer(address account) external view returns (bool) {
        return relayers[account].isActive;
    }

    // ============ Receive Function ============
    /**
     * @notice Reject direct ETH transfers to prevent loss of funds
     * @dev Use depositEth() with a syndDbAccountId to deposit ETH and credit a SyndDB account
     */
    receive() external payable {
        revert("Direct ETH transfers not accepted - use depositEth()");
    }
}
