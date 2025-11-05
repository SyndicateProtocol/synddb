# PLAN_BRIDGE.md - Modular Cross-Chain Message Bridge

## Overview

The Bridge is a modular smart contract system following a "secure by module design" architecture. It processes messages from SyndDB validators and executes them on-chain using a unified module validation system where developers can compose custom pre and post-execution validation logic.

The Bridge acts as the settlement layer for SyndDB, handling:

- Chain crossing token transfers (deposits and withdrawals)
- Cross-chain message passing

## Inspiration

This bridge architecture draws inspiration from proven modular smart contract systems:

- **[Safe Guards](https://docs.safe.global/advanced/smart-account-guards)**: Pre and post-execution hooks for smart account transactions
- **[Zodiac Modules](https://github.com/gnosisguild/zodiac)**: Modular access control patterns
- **[Hyperlane ISMs](https://docs.hyperlane.xyz/docs/protocol/ISM/modular-security)**: Composable module logic with AND/OR patterns, message-type-specific routing
- **[LayerZero DVNs](https://docs.layerzero.network/v2/home/protocol/dvn)**: Flexible verification thresholds and configurable security stacks

### Design Principles

- **Modular design**: Users can add custom before and after hooks
- **Atomic execution**: All stages (pre, core, post) execute in a single transaction - any revert blocks the entire message
- **Validator consensus**: Threshold signatures from validators
- **Composable validation**: Modules can be stacked for complex logic and validation
- **Immutable core**: Bridge logic is fixed, modules are upgradeable

### Modular Architecture

The modular architecture enables advanced validation patterns:

- **Flexible thresholds**: Per-module validation requirements (e.g., "2 of 3 signature validators OR 1 trusted module")
- **Message-type routing**: Different validation rules for WITHDRAW vs DEPOSIT messages (DEPOSIT with zero value indicates a generic call)
- **Default module sets**: Sensible default modules with optional custom overrides

## Architecture

### High-Level Flow

The Bridge processes messages in four distinct stages. **All stages execute atomically in a single transaction** - any module revert (pre or post) will block the entire message from completing.

**Stage 1: Initialization**

- Create message state with messageId, stage, and payload
- Store ValidatorSignatures separately in dedicated mapping
- Mark stage as PreExecution

**Stage 2: PreExecution (Validation)**

- Run all registered pre-execution modules
- Each module receives ProcessingStage and payload
- Modules return bool (true = pass, false = fail)
- ModuleValidator reverts with `ModuleCheckFailed(address module, ProcessingStage stage)` on failure
- Modules can write state for later validation (e.g., record pre-execution NFT count)
- Examples: signature verification, balance checks, withdrawal limits

**Stage 3: Core Execution**

- Execute the core message logic
- Update stage to Executing → PostExecution

**Stage 4: PostExecution (Post-Processing)**

- Run all registered post-execution modules
- Modules can read stored state from pre-execution
- Example: Verify NFT count increased by expected amount
- Modules return bool, ModuleValidator reverts on failure
- Mark stage as Completed
- Examples: supply cap validation, invariant checks

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
- Pre and post validation of messages via modules
- Core execution logic for standard message types
- Replay protection via processed message tracking

## Directory Structure

```
contracts/src/
├── Bridge.sol                          # Core bridge contract (inherits ModuleValidator)
├── ModuleValidator.sol                 # Base contract for module management
│
├── interfaces/
│   ├── IBridge.sol                     # Bridge interface with shared types
│   └── IModuleValidator.sol            # Unified module interface
│
└── modules/
    └── ERC20SupplyValidator.sol           # Example: ERC20 supply cap validation
```

## Core Components

### Bridge.sol

The main bridge contract that orchestrates message processing across all stages.

#### State Variables

```solidity
// In ModuleValidator
EnumerableSet.AddressSet private preModules;
EnumerableSet.AddressSet private postModules;

// In Bridge
mapping(bytes32 messageId => MessageState state) public messageStates;
mapping(bytes32 messageId => ValidatorSignatures sigs) public messageSignatures;
```

#### Data Structures

```solidity
struct MessageState {
    bytes32 messageId;
    ProcessingStage stage;
    bytes payload;
}

struct ValidatorSignatures {
    bytes[] validatorSignatures;
    uint256 submittedAt;
}

enum ProcessingStage {
    NotStarted,
    PreExecution,
    Executing,
    PostExecution,
    Completed,
    Rejected
}
```

#### Core Functions

```solidity
/**
 * Initialize a new message for processing
 * Creates message state and stores validator signatures
 *
 * @param messageId Unique identifier
 * @param payload Message data containing type and parameters
 * @param executionSigs Validator signatures and submission timestamp
 */
function initializeMessage(
    bytes32 messageId,
    bytes calldata payload,
    ValidatorSignatures calldata executionSigs
) public;

/**
 * Execute the message through all validation and execution stages
 * Runs pre-validation, core execution, and post-validation
 *
 * @param messageId The message to execute
 */
function executeMessage(bytes32 messageId) public;

/**
 * Convenience function to initialize and execute in one call
 * Combines initializeMessage() and executeMessage()
 *
 * @param messageId Unique identifier
 * @param payload Message data
 * @param executionSigs Validator signatures
 */
function initializeAndExecuteMessage(
    bytes32 messageId,
    bytes calldata payload,
    ValidatorSignatures calldata executionSigs
) external;

/**
 * Check if a message has been executed
 * Returns true if messageStates[messageId].stage == ProcessingStage.Completed
 *
 * @param messageId The message to check
 * @return bool True if message is completed
 */
function isMessageExecuted(bytes32 messageId) external view returns (bool);
```

#### Module Management

```solidity
/**
 * Add a pre-execution module to the registry
 * Modules execute in the order they are added
 *
 * @param module Address of the module contract
 */
function addPreModule(address module) external;

/**
 * Add a post-execution module to the registry
 * Modules execute in the order they are added
 *
 * @param module Address of the module contract
 */
function addPostModule(address module) external;

/**
 * Remove a pre-execution module from the registry
 *
 * @param module Address of the module to remove
 */
function removePreModule(address module) external;

/**
 * Remove a post-execution module from the registry
 *
 * @param module Address of the module to remove
 */
function removePostModule(address module) external;

/**
 * Get all registered pre-execution modules
 *
 * @return Array of module addresses
 */
function getPreModules() external view returns (address[] memory);

/**
 * Get all registered post-execution modules
 *
 * @return Array of module addresses
 */
function getPostModules() external view returns (address[] memory);
```

### IModuleValidator Interface

A unified interface for both pre and post-execution modules. Modules can determine their behavior based on the ProcessingStage parameter.

```solidity
interface IModuleValidator {
    /**
     * Validate message at a specific processing stage
     *
     * @param stage Current processing stage (PreExecution or PostExecution)
     * @param payload Message data
     * @param executionSigs Validator signatures and submission timestamp
     * @return bool True if validation passes, false otherwise
     */
    function validate(
        IBridge.ProcessingStage stage,
        bytes memory payload,
        IBridge.ValidatorSignatures memory executionSigs
    ) external returns (bool);
}
```

**Key Design Decisions:**

- **Unified Interface**: Single interface for both pre and post-execution modules
- **Stage-Aware**: Modules receive `stage` parameter to determine execution context
- **Bool Return**: Returns bool instead of reverting directly
- **ModuleValidator Reverts**: Parent ModuleValidator contract handles reverting with `ModuleCheckFailed(address module, ProcessingStage stage)`
- **State Mutations Allowed**: Modules can write state (not view-only) to enable pre/post state comparison patterns
- **Example Use Case**: Record NFT supply in pre-execution, verify expected change in post-execution

### Example: ERC20SupplyValidator

A reference implementation demonstrating the module pattern:

```solidity
contract ERC20SupplyValidator is IModuleValidator {
    IERC20 public immutable token;
    uint256 public immutable maxSupply;

    constructor(address _token, uint256 _maxSupply) {
        token = IERC20(_token);
        maxSupply = _maxSupply;
    }

    function validate(
        IBridge.ProcessingStage,
        bytes memory,
        IBridge.ValidatorSignatures memory
    ) external view returns (bool) {
        uint256 currentSupply = token.totalSupply();
        return currentSupply <= maxSupply;
    }
}
```

**Module Characteristics:**

- Can be registered as either pre or post-execution module
- Checks if ERC20 token supply exceeds a maximum threshold
- Returns `false` if supply cap is violated
- Bridge's ModuleValidator reverts with the module address on failure
- Demonstrates simple validation pattern without state storage

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
