# PLAN_BRIDGE.md - Modular Cross-Chain Message Bridge

## Overview

The Bridge is a modular smart contract system that processes messages from SyndDB validators and executes them on-chain. It uses a two-stage module architecture where users can compose custom validation and post-processing logic through PreExecution and PostExecution modules.

The Bridge acts as the settlement layer for SyndDB, handling:

- Chain crossing token transfers (deposits and withdrawals)
- Cross-chain message passing

## Inspiration

This bridge architecture draws inspiration from proven modular smart contract systems:

- **[Safe Guards](https://docs.safe.global/advanced/smart-account-guards)**: Pre and post-execution hooks for smart account transactions
- **[Zodiac Modules](https://github.com/gnosisguild/zodiac)**: Modular access control patterns

### Design Principles

- **Modular design**: Users can add custom before and after hooks
- **Atomic execution**: All stages (pre, core, post) execute in a single transaction - any revert blocks the entire message
- **Validator consensus**: Threshold signatures from validators
- **Composable validation**: Modules can be stacked for complex logic and validation
- **Immutable core**: Bridge logic is fixed, modules are upgradeable

## Architecture

### High-Level Flow

The Bridge processes messages in four distinct stages. **All stages execute atomically in a single transaction** - any module revert (pre or post) will block the entire message from completing.

**Stage 1: Initialization**

- Create message state
- Store payload
- Set initial checkpoint values
- Mark message as active

**Stage 2: PreExecution (Validation)**

- Run PreExecution modules sequentially
- Each module validates the message (validation only, no state mutations)
- Any module can reject the message
- If all pass, advance to execution stage
- Examples: signature verification, balance checks, withdrawal limits

**Stage 3: Core Execution**

- Decode message type from payload
- Execute the operation
- Record success or failure
- Store execution result

**Stage 4: PostExecution (Post-Processing)**

- Run PostExecution modules sequentially
- Process execution results and enforce post-execution invariants (validation only, no state mutations)
- Can revert to block message completion (e.g. supply cap violations)
- Examples: emit events, validate invariants

### Data Flow

```
                    Via Sequencer
                         ↓
                Validators sign message
                that has unique identifier
                         ↓
            ┌─────────────────────────────────────┐
            │  Single Atomic Transaction          │
            │                                     │
            │  1. initializeMessage()             │
            │  2. executePreModules()             │
            │  3. executeMessage()                │
            │  4. executePostModules()            │
            │                                     │
            │  Any revert → entire TX reverts     │
            └─────────────────────────────────────┘
```

### State Transitions

```
NotStarted → PreExecution → Executing → PostExecution → Completed
                  ↓              ↓              ↓
                  └──────────────┴──────────────┘
                            Rejected (any revert is terminal)
```

### Component Structure

The Bridge contract manages:

- Message state and processing stage tracking
- Module registries (PreExecution and PostExecution)
- Core execution logic for standard message types
- Replay protection via processed message tracking

## Directory Structure

```
synddb-bridge/
├── src/
│   ├── Bridge.sol                          # Core bridge contract
│   │
│   ├── interfaces/
│   │   ├── IBridge.sol                     # Bridge interface
│   │   ├── IPreExecutionModule.sol         # PreExecution module interface
│   │   └── IPostExecutionModule.sol        # PostExecution module interface
│   │
│   ├── modules/
│   │   ├── pre/ -> Example PreExecution modules
│   │   │   ├── SignatureValidator.sol      # Validator signature verification
│   │   │   ├── WithdrawalLimitModule.sol   # Daily withdrawal caps
│   │   │   ├── WhitelistModule.sol         # Recipient whitelist checks
│   │   │   └── BalanceCheckModule.sol      # Balance verification
│   │   │
│   │   └── post/ -> Example PostExecution modules
│   │       ├── EventEmitterModule.sol      # Emit detailed events
│   │       ├── SupplyCapModule.sol         # Enforce supply caps
│   │       └── InvariantCheckModule.sol    # Validate post-execution invariants
│   │
│   └── types/
│       ├── MessageTypes.sol                # Message type constants
```

## Core Components

### Bridge.sol

The main bridge contract that orchestrates message processing across all stages.

#### State Variables

```solidity
// Module registries using OpenZeppelin EnumerableSet
EnumerableSet.AddressSet internal preExecutionModules;
EnumerableSet.AddressSet internal postExecutionModules;

// Message tracking
mapping(bytes32 messageId => bool hasBeenProcessed) public processedMessages;
mapping(bytes32 messageId => MessageState messageState) public messageStates;
```

#### Data Structures

```solidity
struct MessageState {
    bytes32 messageId;
    uint256 preModuleCheckpoint;
    uint256 postModuleCheckpoint;
    ProcessingStage stage;
    bytes payload;
    bool executionSuccess;
    bytes executionResult;
}

enum ProcessingStage {
    NotStarted,
    PreExecution,
    Executing,
    PostExecution,
    Completed,
    Rejected
}

struct ExecutionContext {
    address initiator;
    bytes[] validatorSignatures;
    uint256 timestamp;
}
```

#### Core Functions

```solidity
/**
 * Initialize a new message for processing
 * Creates message state and stores payload
 *
 * @param messageId Unique identifier from DA layer
 * @param payload Message data containing type and parameters
 * @param context Execution context with validator signatures
 */
function initializeMessage(
    bytes32 messageId,
    bytes calldata payload,
    ExecutionContext calldata context
) public;

/**
 * Execute PreExecution modules for validation
 * Runs modules from current checkpoint
 * Any module can reject the message
 *
 * @param messageId The message to validate
 */
function executePreModules(bytes32 messageId) public;

/**
 * Execute the core message logic
 * Decodes message type and routes to appropriate handler
 * Records execution success and result
 *
 * @param messageId The message to execute
 */
function executeMessage(bytes32 messageId) public;

/**
 * Execute PostExecution modules for post-processing
 * Runs modules from current checkpoint
 * Module failures do not block message completion
 *
 * @param messageId The message to process
 */
function executePostModules(bytes32 messageId) public;


/**
 * Convenience function to process message through all stages
 * Executes initialize, pre, core, and post in sequence
 *
 * @param messageId Unique identifier from DA layer
 * @param payload Message data
 * @param context Execution context
 */
function processMessageComplete(
    bytes32 messageId,
    bytes calldata payload,
    ExecutionContext calldata context
) external;
```

#### Module Management

```solidity
/**
 * Add a PreExecution module to the registry
 * Modules execute in the order they are added
 *
 * @param module Address of the module contract
 */
function addPreExecutionModule(address module) external;

/**
 * Add a PostExecution module to the registry
 * Modules execute in the order they are added
 *
 * @param module Address of the module contract
 */
function addPostExecutionModule(address module) external;

/**
 * Remove a PreExecution module from the registry
 *
 * @param module Address of the module to remove
 */
function removePreExecutionModule(address module) external;

/**
 * Remove a PostExecution module from the registry
 *
 * @param module Address of the module to remove
 */
function removePostExecutionModule(address module) external;

/**
 * Get all registered PreExecution modules
 *
 * @return Array of module addresses
 */
function getPreExecutionModules() external view returns (address[] memory);

/**
 * Get all registered PostExecution modules
 *
 * @return Array of module addresses
 */
function getPostExecutionModules() external view returns (address[] memory);
```

### IPreExecutionModule Interface

PreExecution modules validate messages before core execution. Modules return bools to enable composable validation logic.

```solidity
interface IPreExecutionModule {
    /// @notice Custom errors for pre-execution failures
    error PreExecutionFailed(bytes32 messageId, uint256 errorCode);

    /**
     * Validate a message before execution
     * Called by Bridge during PreExecution stage
     *
     * @param messageId Unique message identifier from DA layer
     * @param payload Message data containing type and parameters
     * @param context Execution context with validator signatures
     * @return errorCode If false, error code for rejection (0 = success)
     */
    function beforeExecution(
        bytes32 messageId,
        bytes calldata payload,
        ExecutionContext calldata context
    ) external returns (bool shouldExecute, uint256 errorCode);
}
```

### IPostExecutionModule Interface

PostExecution modules process results after core execution and can enforce post-execution invariants. Modules can return false to block message completion (e.g., supply cap violations). They are stateless validators that perform read-only checks. They should not mutate state or make external calls to avoid gas griefing and reentrancy risks.

```solidity
interface IPostExecutionModule {
    /// @notice Custom errors for post-execution failures
    error PostExecutionFailed(bytes32 messageId, uint256 errorCode);

    /**
     * Process results after message execution
     * Called by Bridge during PostExecution stage
     * Can revert to block message completion
     *
     * @param messageId Unique message identifier from DA layer
     * @param success Whether core execution succeeded
     * @param result Execution result data
     * @param context Execution context
     */
    function afterExecution(
        bytes32 messageId,
        bool success,
        bytes calldata result,
        ExecutionContext calldata context
    ) external;
}
```

## Message Format Specification

### Payload Structure

All messages follow a standard format with a 4-byte type selector followed by ABI-encoded parameters.

```
Payload Format:
[0:4]   bytes4  - Message type selector (function signature hash)
[4:end] bytes   - ABI-encoded parameters specific to message type
```

### Standard Message Types

**Withdrawal:**

Withdraws tokens from the bridge to a recipient on the destination chain.

```solidity
bytes4 constant WITHDRAW = bytes4(keccak256("withdraw(address,address,uint256)"));
// Parameters: (address token, address recipient, uint256 amount)
```

**Deposit:**

Deposits tokens into the bridge from a sender on the source chain. When the deposit amount is zero, the message is treated as a generic contract call with a payload to execute on the target contract.

```solidity
bytes4 constant DEPOSIT = bytes4(keccak256("deposit(address,address,uint256)"));
// Parameters: (address token, address sender, uint256 amount)
```
