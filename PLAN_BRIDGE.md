# PLAN_BRIDGE.md - Modular Cross-Chain Message Bridge

## Overview

The Bridge is a modular smart contract system that processes messages from SyndDB validators and executes them on-chain. It uses a two-stage module architecture where users can compose custom validation and post-processing logic through PreExecution and PostExecution modules.

The Bridge acts as the settlement layer for SyndDB, handling:

- Asset withdrawals from the bridge contract
- Generic contract calls to any address
- Cross-chain message passing
- Result callbacks to SyndDB infrastructure

### Design Principles

- Modular design: Users can add custom before and after hooks
- Checkpoint-based processing: No unbounded loops, predictable gas usage
- Validator consensus: Threshold signatures from validators
- Immutable core: Bridge logic is fixed, modules are upgradeable
- DA layer integration: Messages published to DA before bridge execution, messageId sourced from DA layer

## Architecture

### High-Level Flow

The Bridge processes messages in four distinct stages. Each stage is called explicitly and can be executed across multiple transactions for gas control.

**Stage 1: Initialization**

- Create message state
- Store payload
- Set initial checkpoint values
- Mark message as active

**Stage 2: PreExecution (Validation)**

- Run PreExecution modules sequentially
- Each module validates the message
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
- Process execution results
- Examples: emit events, send callbacks, collect fees

### Data Flow

```
                     Validators
                         ↓
                    Sign messageId
                         ↓
            bridge.initializeMessage(messageId, payload, context)
                         ↓
            bridge.executePreModules(messageId)
                         ↓
            bridge.executeMessage(messageId)
                         ↓
            bridge.executePostModules(messageId)
```

### Component Structure

The Bridge contract manages:

- Message state and processing stage tracking
- Module registries (PreExecution and PostExecution)
- Checkpoint-based processing for gas efficiency
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
│   │       ├── CallbackModule.sol          # Send confirmation to SyndDB
│   │       └── FeeCollectorModule.sol      # Collect protocol fees
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
    bool executionSuccess; // optional as it can be inferred from stage
    uint256 createdAt; // is it needed?
}

enum ProcessingStage {
    NotStarted,
    PreExecution,
    Executing,
    PostExecution,
    Completed
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
) external;

/**
 * Execute PreExecution modules for validation
 * Runs modules from current checkpoint
 * Any module can reject the message
 *
 * @param messageId The message to validate
 */
function executePreModules(bytes32 messageId) external;

/**
 * Execute the core message logic
 * Decodes message type and routes to appropriate handler
 * Records execution success and result
 *
 * @param messageId The message to execute
 */
function executeMessage(bytes32 messageId) external;

/**
 * Execute PostExecution modules for post-processing
 * Runs modules from current checkpoint
 * Module failures do not block message completion
 *
 * @param messageId The message to process
 */
function executePostModules(bytes32 messageId) external;

/// *** OPTIONAL HELPER. Need to experiment if gas limits allow. *** ///

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

PreExecution modules validate messages before core execution. Any module returning false will reject the message.

```solidity
interface IPreExecutionModule {
    /**
     * Validate a message before execution
     * Called by Bridge during PreExecution stage
     *
     * @param messageId Unique message identifier from DA layer
     * @param payload Message data containing type and parameters
     * @param context Execution context with validator signatures
     * @return shouldExecute True if validation passes, false to reject
     * @return reason If false, explanation for rejection
     */
    function beforeExecution(
        bytes32 messageId,
        bytes calldata payload,
        ExecutionContext calldata context
    ) external returns (bool shouldExecute, string memory reason);
}
```

### IPostExecutionModule Interface

PostExecution modules process results after core execution. Module failures do not block message completion.

```solidity
interface IPostExecutionModule {
    /**
     * Process results after message execution
     * Called by Bridge during PostExecution stage
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

## Further Considerations

- Have a Batch contract that the Bridge can call to batch multiple messages together for gas efficiency.
- Should we add processMessageComplete() as a convenience function to process all stages in one call within Bridge.sol?
- is a createdAt timestamp needed in MessageState struct? // consider use cases for message age.
