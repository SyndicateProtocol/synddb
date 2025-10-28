// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";
import "@openzeppelin/contracts/security/ReentrancyGuard.sol";
import "@openzeppelin/contracts/security/Pausable.sol";
import "@openzeppelin/contracts/access/Ownable.sol";
import "@openzeppelin/contracts/utils/cryptography/ECDSA.sol";
import "@openzeppelin/contracts/utils/cryptography/EIP712.sol";

/**
 * @title SyndDBBridge
 * @notice Bridge contract for SyndDB enabling deposits, withdrawals, and batch settlements
 * @dev Supports complex orderbook operations with m-of-n validator signatures
 */
contract SyndDBBridge is ReentrancyGuard, Pausable, Ownable, EIP712 {
    using SafeERC20 for IERC20;
    using ECDSA for bytes32;

    // ============ Constants ============
    uint256 public constant BASIS_POINTS = 10000;
    uint256 public constant MAX_VALIDATORS = 20;
    uint256 public constant MIN_VALIDATORS = 3;

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
    }

    mapping(address => Validator) public validators;
    address[] public validatorList;
    uint256 public requiredSignatures; // m in m-of-n
    address public sequencer;

    // Deposit tracking
    struct DepositRecord {
        address depositor;
        address token;
        uint256 amount;
        uint256 blockNumber;
        bytes32 syndDbAccountId;
    }

    uint256 public totalDeposits;
    mapping(uint256 => DepositRecord) public deposits;
    mapping(address => mapping(address => uint256)) public userDeposits; // user => token => amount

    // Withdrawal tracking
    mapping(uint256 => bool) public processedNonces;
    uint256 public currentNonce;

    // Balance updates for batch settlements
    struct BalanceUpdate {
        address account;
        address token;
        int256 delta; // Can be negative for debits
    }

    // Circuit breakers
    mapping(address => uint256) public dailyWithdrawalLimit; // Per token
    mapping(address => uint256) public dailyWithdrawn; // token => amount withdrawn today
    mapping(address => uint256) public lastWithdrawalDay; // token => day number

    mapping(address => mapping(address => uint256)) public userDailyWithdrawn; // user => token => amount
    mapping(address => mapping(address => uint256)) public userLastWithdrawalDay; // user => token => day
    uint256 public globalDailyLimit = 10_000_000 * 10**18; // $10M default
    uint256 public userDailyLimit = 1_000_000 * 10**18; // $1M default

    // Fees
    uint256 public depositFeeBps = 10; // 0.1%
    uint256 public withdrawalFeeBps = 30; // 0.3%
    address public feeRecipient;
    mapping(address => uint256) public accumulatedFees; // token => amount

    // Emergency controls
    bool public depositsEnabled = true;
    bool public withdrawalsEnabled = true;
    uint256 public emergencyPauseDeadline;

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

    event FeesCollected(address indexed token, uint256 amount, address indexed recipient);
    event EmergencyPause(uint256 deadline);

    // ============ Modifiers ============
    modifier onlySequencer() {
        require(msg.sender == sequencer, "Not sequencer");
        _;
    }

    modifier whenDepositsEnabled() {
        require(depositsEnabled, "Deposits disabled");
        _;
    }

    modifier whenWithdrawalsEnabled() {
        require(withdrawalsEnabled, "Withdrawals disabled");
        _;
    }

    // ============ Constructor ============
    constructor(
        address _sequencer,
        address[] memory _initialValidators,
        uint256 _requiredSignatures,
        address _feeRecipient
    ) EIP712("SyndDBBridge", "1") {
        require(_sequencer != address(0), "Invalid sequencer");
        require(_feeRecipient != address(0), "Invalid fee recipient");
        require(_initialValidators.length >= MIN_VALIDATORS, "Too few validators");
        require(_initialValidators.length <= MAX_VALIDATORS, "Too many validators");
        require(_requiredSignatures <= _initialValidators.length, "Invalid signature requirement");
        require(_requiredSignatures >= 1, "At least 1 signature required");

        sequencer = _sequencer;
        requiredSignatures = _requiredSignatures;
        feeRecipient = _feeRecipient;

        for (uint256 i = 0; i < _initialValidators.length; i++) {
            require(_initialValidators[i] != address(0), "Invalid validator");
            validators[_initialValidators[i]] = Validator({
                publicKey: _initialValidators[i],
                attestationHash: bytes32(0),
                isActive: true,
                addedAt: block.timestamp
            });
            validatorList.push(_initialValidators[i]);
        }
    }

    // ============ Deposit Functions ============

    /**
     * @notice Deposit ERC20 tokens to the bridge
     * @param token The ERC20 token address
     * @param amount The amount to deposit
     * @param syndDbAccountId The account ID in SyndDB to credit
     */
    function deposit(
        address token,
        uint256 amount,
        bytes32 syndDbAccountId
    ) external nonReentrant whenNotPaused whenDepositsEnabled {
        require(token != address(0), "Invalid token");
        require(amount > 0, "Zero amount");
        require(syndDbAccountId != bytes32(0), "Invalid SyndDB account");

        // Calculate fees
        uint256 fee = (amount * depositFeeBps) / BASIS_POINTS;
        uint256 amountAfterFee = amount - fee;

        // Transfer tokens from user
        IERC20(token).safeTransferFrom(msg.sender, address(this), amount);

        // Track deposit
        uint256 depositId = totalDeposits++;
        deposits[depositId] = DepositRecord({
            depositor: msg.sender,
            token: token,
            amount: amountAfterFee,
            blockNumber: block.number,
            syndDbAccountId: syndDbAccountId
        });

        userDeposits[msg.sender][token] += amountAfterFee;
        accumulatedFees[token] += fee;

        emit Deposit(msg.sender, token, amountAfterFee, syndDbAccountId, depositId);
    }

    /**
     * @notice Deposit ETH to the bridge
     * @param syndDbAccountId The account ID in SyndDB to credit
     */
    function depositETH(bytes32 syndDbAccountId) external payable nonReentrant whenNotPaused whenDepositsEnabled {
        require(msg.value > 0, "Zero amount");
        require(syndDbAccountId != bytes32(0), "Invalid SyndDB account");

        // Calculate fees
        uint256 fee = (msg.value * depositFeeBps) / BASIS_POINTS;
        uint256 amountAfterFee = msg.value - fee;

        // Track deposit
        uint256 depositId = totalDeposits++;
        deposits[depositId] = DepositRecord({
            depositor: msg.sender,
            token: address(0), // ETH represented as address(0)
            amount: amountAfterFee,
            blockNumber: block.number,
            syndDbAccountId: syndDbAccountId
        });

        userDeposits[msg.sender][address(0)] += amountAfterFee;
        accumulatedFees[address(0)] += fee;

        emit Deposit(msg.sender, address(0), amountAfterFee, syndDbAccountId, depositId);
    }

    // ============ Withdrawal Functions ============

    /**
     * @notice Process a withdrawal with validator signatures
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
    ) external nonReentrant whenNotPaused whenWithdrawalsEnabled {
        // Check deadline
        require(block.timestamp <= deadline, "Withdrawal expired");
        require(!processedNonces[nonce], "Nonce already processed");
        require(validatorSignatures.length >= requiredSignatures, "Insufficient signatures");

        // Check circuit breakers
        require(checkWithdrawalLimits(token, amount, recipient), "Withdrawal limit exceeded");

        // Construct the withdrawal message
        bytes32 structHash = keccak256(abi.encode(
            WITHDRAWAL_TYPEHASH,
            nonce,
            recipient,
            token,
            amount,
            deadline
        ));

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

        // Calculate fees
        uint256 fee = (amount * withdrawalFeeBps) / BASIS_POINTS;
        uint256 amountAfterFee = amount - fee;
        accumulatedFees[token] += fee;

        // Update circuit breaker tracking
        updateWithdrawalTracking(token, amount, recipient);

        // Execute withdrawal
        if (token == address(0)) {
            // ETH withdrawal
            (bool success, ) = recipient.call{value: amountAfterFee}("");
            require(success, "ETH transfer failed");
        } else {
            // ERC20 withdrawal
            IERC20(token).safeTransfer(recipient, amountAfterFee);
        }

        emit Withdrawal(recipient, token, amountAfterFee, nonce);
    }

    /**
     * @notice Process a batch settlement for orderbook operations
     * @param nonce The unique nonce for this settlement
     * @param stateRoot The merkle root of balance updates
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
    ) external nonReentrant whenNotPaused whenWithdrawalsEnabled {
        require(block.timestamp <= deadline, "Settlement expired");
        require(!processedNonces[nonce], "Nonce already processed");
        require(validatorSignatures.length >= requiredSignatures, "Insufficient signatures");
        require(updates.length > 0 && updates.length <= 100, "Invalid update count");

        // Verify state root matches updates
        bytes32 calculatedRoot = calculateMerkleRoot(updates);
        require(calculatedRoot == stateRoot, "State root mismatch");

        // Construct the settlement message
        bytes32 structHash = keccak256(abi.encode(
            BATCH_SETTLEMENT_TYPEHASH,
            nonce,
            stateRoot,
            deadline
        ));

        bytes32 messageHash = _hashTypedDataV4(structHash);

        // Verify sequencer signature
        address recoveredSequencer = messageHash.recover(sequencerSignature);
        require(recoveredSequencer == sequencer, "Invalid sequencer signature");

        // Verify validator signatures (similar to withdrawal)
        uint256 validSignatures = verifyValidatorSignatures(messageHash, validatorSignatures);
        require(validSignatures >= requiredSignatures, "Insufficient valid signatures");

        // Mark nonce as processed
        processedNonces[nonce] = true;

        // Process all balance updates atomically
        for (uint256 i = 0; i < updates.length; i++) {
            BalanceUpdate memory update = updates[i];

            if (update.delta > 0) {
                // Credit to user (withdrawal from SyndDB)
                uint256 amount = uint256(update.delta);

                // Check circuit breakers for withdrawals
                require(checkWithdrawalLimits(update.token, amount, update.account), "Withdrawal limit exceeded");
                updateWithdrawalTracking(update.token, amount, update.account);

                if (update.token == address(0)) {
                    (bool success, ) = update.account.call{value: amount}("");
                    require(success, "ETH transfer failed");
                } else {
                    IERC20(update.token).safeTransfer(update.account, amount);
                }
            } else if (update.delta < 0) {
                // Debit from user (should have been pre-deposited)
                uint256 amount = uint256(-update.delta);
                require(userDeposits[update.account][update.token] >= amount, "Insufficient deposit balance");
                userDeposits[update.account][update.token] -= amount;
            }
        }

        emit BatchSettlement(stateRoot, updates.length, nonce);
    }

    // ============ Validator Management ============

    /**
     * @notice Add a new validator with TEE attestation
     * @param validator The validator address
     * @param attestationHash The hash of the TEE attestation
     */
    function addValidator(address validator, bytes32 attestationHash) external onlyOwner {
        require(validator != address(0), "Invalid validator");
        require(!validators[validator].isActive, "Already active");
        require(validatorList.length < MAX_VALIDATORS, "Too many validators");

        validators[validator] = Validator({
            publicKey: validator,
            attestationHash: attestationHash,
            isActive: true,
            addedAt: block.timestamp
        });

        validatorList.push(validator);

        emit ValidatorAdded(validator, attestationHash);
    }

    /**
     * @notice Remove a validator
     * @param validator The validator to remove
     * @param reason The reason for removal
     */
    function removeValidator(address validator, string memory reason) external onlyOwner {
        require(validators[validator].isActive, "Not active");
        require(validatorList.length - 1 >= MIN_VALIDATORS, "Too few validators");
        require(validatorList.length - 1 >= requiredSignatures, "Would break signature requirement");

        validators[validator].isActive = false;

        // Remove from list
        for (uint256 i = 0; i < validatorList.length; i++) {
            if (validatorList[i] == validator) {
                validatorList[i] = validatorList[validatorList.length - 1];
                validatorList.pop();
                break;
            }
        }

        emit ValidatorRemoved(validator, reason);
    }

    /**
     * @notice Update validator attestation
     * @param validator The validator address
     * @param newAttestationHash The new attestation hash
     */
    function updateValidatorAttestation(address validator, bytes32 newAttestationHash) external onlyOwner {
        require(validators[validator].isActive, "Not active");
        validators[validator].attestationHash = newAttestationHash;
        emit ValidatorUpdated(validator, newAttestationHash);
    }

    // ============ Circuit Breaker Functions ============

    /**
     * @notice Check if withdrawal is within limits
     */
    function checkWithdrawalLimits(
        address token,
        uint256 amount,
        address user
    ) internal view returns (bool) {
        uint256 currentDay = block.timestamp / 86400;

        // Check global daily limit for token
        uint256 tokenDailyWithdrawn = dailyWithdrawn[token];
        if (currentDay != lastWithdrawalDay[token]) {
            tokenDailyWithdrawn = 0;
        }
        if (tokenDailyWithdrawn + amount > globalDailyLimit) {
            return false;
        }

        // Check user daily limit
        uint256 userTokenDailyWithdrawn = userDailyWithdrawn[user][token];
        if (currentDay != userLastWithdrawalDay[user][token]) {
            userTokenDailyWithdrawn = 0;
        }
        if (userTokenDailyWithdrawn + amount > userDailyLimit) {
            return false;
        }

        return true;
    }

    /**
     * @notice Update withdrawal tracking for circuit breakers
     */
    function updateWithdrawalTracking(
        address token,
        uint256 amount,
        address user
    ) internal {
        uint256 currentDay = block.timestamp / 86400;

        // Update global tracking
        if (currentDay != lastWithdrawalDay[token]) {
            dailyWithdrawn[token] = amount;
            lastWithdrawalDay[token] = currentDay;
        } else {
            dailyWithdrawn[token] += amount;
        }

        // Update user tracking
        if (currentDay != userLastWithdrawalDay[user][token]) {
            userDailyWithdrawn[user][token] = amount;
            userLastWithdrawalDay[user][token] = currentDay;
        } else {
            userDailyWithdrawn[user][token] += amount;
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
        require(newRequired > 0 && newRequired <= validatorList.length, "Invalid requirement");
        uint256 oldRequired = requiredSignatures;
        requiredSignatures = newRequired;
        emit RequiredSignaturesUpdated(oldRequired, newRequired);
    }

    /**
     * @notice Set withdrawal limits
     */
    function setWithdrawalLimits(
        uint256 _globalDailyLimit,
        uint256 _userDailyLimit
    ) external onlyOwner {
        globalDailyLimit = _globalDailyLimit;
        userDailyLimit = _userDailyLimit;
    }

    /**
     * @notice Set fee parameters
     */
    function setFeeParameters(
        uint256 _depositFeeBps,
        uint256 _withdrawalFeeBps,
        address _feeRecipient
    ) external onlyOwner {
        require(_depositFeeBps <= 100, "Deposit fee too high"); // Max 1%
        require(_withdrawalFeeBps <= 100, "Withdrawal fee too high"); // Max 1%
        require(_feeRecipient != address(0), "Invalid recipient");

        depositFeeBps = _depositFeeBps;
        withdrawalFeeBps = _withdrawalFeeBps;
        feeRecipient = _feeRecipient;
    }

    /**
     * @notice Collect accumulated fees
     */
    function collectFees(address token) external {
        uint256 amount = accumulatedFees[token];
        require(amount > 0, "No fees to collect");

        accumulatedFees[token] = 0;

        if (token == address(0)) {
            (bool success, ) = feeRecipient.call{value: amount}("");
            require(success, "ETH transfer failed");
        } else {
            IERC20(token).safeTransfer(feeRecipient, amount);
        }

        emit FeesCollected(token, amount, feeRecipient);
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
    function verifyValidatorSignatures(
        bytes32 messageHash,
        bytes[] memory signatures
    ) internal view returns (uint256) {
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
        return validatorList.length;
    }

    /**
     * @notice Get all validators
     */
    function getValidators() external view returns (address[] memory) {
        return validatorList;
    }

    /**
     * @notice Check if an address is an active validator
     */
    function isValidator(address account) external view returns (bool) {
        return validators[account].isActive;
    }

    /**
     * @notice Get user's deposited balance
     */
    function getUserDepositBalance(address user, address token) external view returns (uint256) {
        return userDeposits[user][token];
    }

    /**
     * @notice Get accumulated fees for a token
     */
    function getAccumulatedFees(address token) external view returns (uint256) {
        return accumulatedFees[token];
    }

    // ============ Receive Function ============
    receive() external payable {
        // Accept ETH for withdrawals
    }
}