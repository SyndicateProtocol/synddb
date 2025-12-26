// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

/**
 * @title PredictionMarket
 * @notice Binary prediction market receiving state via Bridge messages
 * @dev All write operations are restricted to the Bridge contract.
 *      This contract demonstrates the message-passing paradigm where:
 *      - All state mutations come from validated Bridge messages
 *      - Clients read state directly from the contract
 *      - Complex operations happen off-chain, only final state goes on-chain
 */
contract PredictionMarket {
    // =========================================================================
    // Types
    // =========================================================================

    struct Market {
        string question;
        uint256 resolutionTime;
        bool resolved;
        uint8 winningOutcome; // 0=NO, 1=YES
        uint256 totalYesShares;
        uint256 totalNoShares;
        uint256 createdAt;
        uint256 resolvedAt;
    }

    struct Position {
        uint256 shares;
        uint256 costBasis;
        bool claimed;
    }

    // =========================================================================
    // Constants
    // =========================================================================

    /// @notice Fixed price per share in cents (50/50 pricing)
    uint256 public constant SHARE_PRICE = 50;

    /// @notice Payout per winning share in cents
    uint256 public constant PAYOUT_PER_SHARE = 100;

    // =========================================================================
    // State
    // =========================================================================

    /// @notice Markets by ID (keccak256 of string ID)
    mapping(bytes32 => Market) public markets;

    /// @notice Positions: marketId => user => outcome => Position
    mapping(bytes32 => mapping(address => mapping(uint8 => Position))) public positions;

    /// @notice User balances in cents
    mapping(address => uint256) public balances;

    /// @notice Bridge contract address (only can call state-changing functions)
    address public immutable bridge;

    /// @notice Contract owner
    address public owner;

    // =========================================================================
    // Events
    // =========================================================================

    event MarketCreated(
        bytes32 indexed marketId,
        string question,
        uint256 resolutionTime
    );

    event MarketResolved(
        bytes32 indexed marketId,
        uint8 outcome
    );

    event Deposit(
        address indexed user,
        uint256 amount
    );

    event SharesPurchased(
        bytes32 indexed marketId,
        address indexed user,
        uint8 outcome,
        uint256 shares
    );

    event SharesSold(
        bytes32 indexed marketId,
        address indexed user,
        uint8 outcome,
        uint256 shares
    );

    event WinningsClaimed(
        bytes32 indexed marketId,
        address indexed user,
        uint256 amount
    );

    // =========================================================================
    // Errors
    // =========================================================================

    error Unauthorized();
    error MarketAlreadyExists(bytes32 marketId);
    error MarketNotFound(bytes32 marketId);
    error MarketNotResolved(bytes32 marketId);
    error MarketAlreadyResolved(bytes32 marketId);
    error InvalidOutcome(uint8 outcome);
    error InsufficientBalance(uint256 required, uint256 available);
    error InsufficientShares(uint256 required, uint256 available);
    error AlreadyClaimed(bytes32 marketId, address user);
    error NoWinningPosition(bytes32 marketId, address user);

    // =========================================================================
    // Modifiers
    // =========================================================================

    modifier onlyBridge() {
        if (msg.sender != bridge) revert Unauthorized();
        _;
    }

    modifier onlyOwner() {
        if (msg.sender != owner) revert Unauthorized();
        _;
    }

    // =========================================================================
    // Constructor
    // =========================================================================

    constructor(address _bridge) {
        bridge = _bridge;
        owner = msg.sender;
    }

    // =========================================================================
    // Bridge-only functions (called via validated messages)
    // =========================================================================

    /**
     * @notice Create a new prediction market
     * @param marketId Unique market identifier
     * @param question The question being predicted
     * @param resolutionTime Unix timestamp when market can be resolved
     */
    function createMarket(
        bytes32 marketId,
        string calldata question,
        uint256 resolutionTime
    ) external onlyBridge {
        if (markets[marketId].createdAt != 0) {
            revert MarketAlreadyExists(marketId);
        }

        markets[marketId] = Market({
            question: question,
            resolutionTime: resolutionTime,
            resolved: false,
            winningOutcome: 0,
            totalYesShares: 0,
            totalNoShares: 0,
            createdAt: block.timestamp,
            resolvedAt: 0
        });

        emit MarketCreated(marketId, question, resolutionTime);
    }

    /**
     * @notice Deposit funds for a user
     * @param user User address to credit
     * @param amount Amount in cents
     */
    function deposit(address user, uint256 amount) external onlyBridge {
        balances[user] += amount;
        emit Deposit(user, amount);
    }

    /**
     * @notice Buy shares in a market outcome
     * @param marketId Market to buy in
     * @param user User buying shares
     * @param outcome Outcome to buy (0=NO, 1=YES)
     * @param shares Number of shares to buy
     */
    function buyShares(
        bytes32 marketId,
        address user,
        uint8 outcome,
        uint256 shares
    ) external onlyBridge {
        if (outcome > 1) revert InvalidOutcome(outcome);
        if (markets[marketId].createdAt == 0) revert MarketNotFound(marketId);
        if (markets[marketId].resolved) revert MarketAlreadyResolved(marketId);

        uint256 cost = shares * SHARE_PRICE;
        if (balances[user] < cost) {
            revert InsufficientBalance(cost, balances[user]);
        }

        // Deduct balance
        balances[user] -= cost;

        // Update position
        Position storage pos = positions[marketId][user][outcome];
        pos.shares += shares;
        pos.costBasis += cost;

        // Update market totals
        if (outcome == 1) {
            markets[marketId].totalYesShares += shares;
        } else {
            markets[marketId].totalNoShares += shares;
        }

        emit SharesPurchased(marketId, user, outcome, shares);
    }

    /**
     * @notice Sell shares in a market outcome
     * @param marketId Market to sell in
     * @param user User selling shares
     * @param outcome Outcome to sell (0=NO, 1=YES)
     * @param shares Number of shares to sell
     */
    function sellShares(
        bytes32 marketId,
        address user,
        uint8 outcome,
        uint256 shares
    ) external onlyBridge {
        if (outcome > 1) revert InvalidOutcome(outcome);
        if (markets[marketId].createdAt == 0) revert MarketNotFound(marketId);
        if (markets[marketId].resolved) revert MarketAlreadyResolved(marketId);

        Position storage pos = positions[marketId][user][outcome];
        if (pos.shares < shares) {
            revert InsufficientShares(shares, pos.shares);
        }

        uint256 proceeds = shares * SHARE_PRICE;

        // Credit balance
        balances[user] += proceeds;

        // Update position (proportional cost basis reduction)
        uint256 costReduction = (pos.costBasis * shares) / pos.shares;
        pos.shares -= shares;
        pos.costBasis -= costReduction;

        // Update market totals
        if (outcome == 1) {
            markets[marketId].totalYesShares -= shares;
        } else {
            markets[marketId].totalNoShares -= shares;
        }

        emit SharesSold(marketId, user, outcome, shares);
    }

    /**
     * @notice Resolve a market with the winning outcome
     * @param marketId Market to resolve
     * @param outcome Winning outcome (0=NO, 1=YES)
     */
    function resolveMarket(bytes32 marketId, uint8 outcome) external onlyBridge {
        if (outcome > 1) revert InvalidOutcome(outcome);
        if (markets[marketId].createdAt == 0) revert MarketNotFound(marketId);
        if (markets[marketId].resolved) revert MarketAlreadyResolved(marketId);

        markets[marketId].resolved = true;
        markets[marketId].winningOutcome = outcome;
        markets[marketId].resolvedAt = block.timestamp;

        emit MarketResolved(marketId, outcome);
    }

    // =========================================================================
    // User functions (permissionless)
    // =========================================================================

    /**
     * @notice Claim winnings from a resolved market
     * @param marketId Market to claim from
     */
    function claimWinnings(bytes32 marketId) external {
        Market storage market = markets[marketId];
        if (!market.resolved) revert MarketNotResolved(marketId);

        Position storage pos = positions[marketId][msg.sender][market.winningOutcome];
        if (pos.claimed) revert AlreadyClaimed(marketId, msg.sender);
        if (pos.shares == 0) revert NoWinningPosition(marketId, msg.sender);

        uint256 payout = pos.shares * PAYOUT_PER_SHARE;
        pos.claimed = true;

        // Credit balance (user can withdraw later)
        balances[msg.sender] += payout;

        emit WinningsClaimed(marketId, msg.sender, payout);
    }

    // =========================================================================
    // View functions
    // =========================================================================

    /**
     * @notice Get market info
     */
    function getMarket(bytes32 marketId) external view returns (Market memory) {
        return markets[marketId];
    }

    /**
     * @notice Get user position in a market
     */
    function getPosition(
        bytes32 marketId,
        address user,
        uint8 outcome
    ) external view returns (Position memory) {
        return positions[marketId][user][outcome];
    }

    /**
     * @notice Get user balance
     */
    function getBalance(address user) external view returns (uint256) {
        return balances[user];
    }

    /**
     * @notice Check if a market exists
     */
    function marketExists(bytes32 marketId) external view returns (bool) {
        return markets[marketId].createdAt != 0;
    }

    /**
     * @notice Get implied probability for YES outcome (based on share distribution)
     */
    function getImpliedProbability(bytes32 marketId) external view returns (uint256 yesPct, uint256 noPct) {
        Market storage market = markets[marketId];
        uint256 total = market.totalYesShares + market.totalNoShares;
        if (total == 0) {
            return (50, 50); // Default 50/50
        }
        yesPct = (market.totalYesShares * 100) / total;
        noPct = 100 - yesPct;
    }

    // =========================================================================
    // Owner functions
    // =========================================================================

    /**
     * @notice Transfer ownership
     */
    function transferOwnership(address newOwner) external onlyOwner {
        owner = newOwner;
    }

    /**
     * @notice Receive ETH (for funding the contract)
     */
    receive() external payable {}
}
