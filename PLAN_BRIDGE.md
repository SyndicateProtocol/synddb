# PLAN_BRIDGE.md - Generic Message Passing via Smart Contracts

## Overview

The SyndDB bridge is primarily a **Solidity smart contract** that mirrors the offchain message tables maintained by validators. Unlike traditional bridges that hardcode specific operations, this contract allows applications to define their own message schemas in SQL tables, which automatically map to smart contract ABIs. Validators monitor these tables in the replicated SQLite database and submit messages to the Bridge.sol contract with their signatures.

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                     Application (Any Language)                  │
│  Writes to Message Tables:                                      │
│  - outbound_withdrawals                                          │
│  - outbound_messages                                             │
│  - outbound_calls                                                │
│  Application code just writes SQL, nothing blockchain-specific   │
└─────────────────────────────────────────────────────────────────┘
                              ↓ (via Sidecar → DA)
┌─────────────────────────────────────────────────────────────────┐
│                    Validator Network (TEEs)                     │
│  Validators are just read replicas in validator mode:           │
│  1. Sync message tables from DA layers (normal replication)     │
│  2. Detect new pending messages via SQL queries                 │
│  3. Sign messages and coordinate with other validators          │
│  4. Submit to Bridge.sol with multi-sig                         │
│  5. Listen for inbound events from Bridge.sol                   │
│  6. Write inbound messages to inbound_* tables                  │
└─────────────────────────────────────────────────────────────────┘
                              ↓
┌─────────────────────────────────────────────────────────────────┐
│              Settlement Chain (Ethereum/L2)                     │
│  ┌───────────────────────────────────────────────────────────┐ │
│  │                    Bridge.sol                              │ │
│  │  Core Logic (all in Solidity):                            │ │
│  │  - Verify validator signatures on messages                │ │
│  │  - Execute withdrawals (transfer tokens/ETH)              │ │
│  │  - Execute arbitrary calls to other contracts             │ │
│  │  - Process oracle requests                                │ │
│  │  - Enforce circuit breakers and limits                    │ │
│  │  - Emit events for inbound messages                       │ │
│  │                                                            │ │
│  │  All signature verification, multi-sig, message           │ │
│  │  processing happens ON-CHAIN in Solidity                  │ │
│  └───────────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────────┘
                              ↓ (Events)
┌─────────────────────────────────────────────────────────────────┐
│                 Validator Inbound Processing                    │
│  Validators listen for Bridge.sol events:                       │
│  - Parse InboundMessage events                                  │
│  - Write to application's inbound_* tables (SQL INSERT)         │
│  - Application processes via normal SQL queries                 │
└─────────────────────────────────────────────────────────────────┘
```

**Key Principle**: Bridge logic lives in **Solidity**. Validators just:
- Query SQLite for pending messages
- Sign messages
- Submit transactions to Bridge.sol
- Listen for events and write to inbound tables

No complex Rust signature aggregation or consensus logic - that's all on-chain.

## Smart Contract Implementation

### Core Bridge Contract

```solidity
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import "@openzeppelin/contracts-upgradeable/proxy/utils/Initializable.sol";
import "@openzeppelin/contracts-upgradeable/access/AccessControlUpgradeable.sol";
import "@openzeppelin/contracts-upgradeable/security/PausableUpgradeable.sol";
import "@openzeppelin/contracts-upgradeable/security/ReentrancyGuardUpgradeable.sol";
import "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";

/**
 * @title Bridge
 * @notice Generic message passing bridge that mirrors SQLite message tables
 * @dev All message processing logic lives in this contract
 */
contract Bridge is
    Initializable,
    AccessControlUpgradeable,
    PausableUpgradeable,
    ReentrancyGuardUpgradeable
{
    using SafeERC20 for IERC20;

    // ============ Roles ============

    bytes32 public constant VALIDATOR_ROLE = keccak256("VALIDATOR_ROLE");
    bytes32 public constant ADMIN_ROLE = keccak256("ADMIN_ROLE");

    // ============ Types ============

    enum MessageType {
        WITHDRAWAL,      // Withdraw tokens/ETH to external address
        DEPOSIT,         // Handled via direct contract calls, not messages
        CALL,            // Arbitrary contract call
        ORACLE_REQUEST,  // Request data from oracle
        ORACLE_RESPONSE, // Oracle response
        GOVERNANCE,      // Governance action
        CUSTOM           // Custom handler
    }

    struct Message {
        uint256 id;              // Message ID from SQL table
        MessageType messageType; // Type of message
        bytes32 schemaHash;      // Hash of SQL table schema
        bytes payload;           // ABI-encoded data from table row
        uint256 nonce;           // Sequence number for replay protection
        uint256 timestamp;       // When message was created
    }

    // ============ State Variables ============

    // Validator management
    mapping(address => bool) public validators;
    address[] public validatorList;
    uint256 public validatorThreshold;  // Min signatures required

    // Message tracking
    mapping(uint256 => bool) public processedMessages;
    mapping(bytes32 => uint256) public schemaNonces;  // Per-schema nonce
    uint256 public currentNonce;  // Global nonce for inbound messages

    // Custom message handlers
    mapping(bytes32 => address) public schemaToHandler;

    // Circuit breakers and limits
    uint256 public dailyWithdrawalLimit;
    uint256 public dailyWithdrawn;
    uint256 public lastWithdrawalReset;
    uint256 public maxMessageSize;
    uint256 public minValidators;

    // ============ Events ============

    event MessageProcessed(
        uint256 indexed messageId,
        MessageType indexed messageType,
        bytes32 schemaHash,
        address indexed submitter
    );

    event WithdrawalExecuted(
        uint256 indexed messageId,
        address indexed recipient,
        address indexed token,
        uint256 amount
    );

    event CallExecuted(
        uint256 indexed messageId,
        address indexed target,
        bool success,
        bytes returnData
    );

    event InboundMessage(
        uint256 indexed nonce,
        address indexed sender,
        bytes32 indexed targetSchema,
        bytes payload
    );

    event ValidatorAdded(address indexed validator);
    event ValidatorRemoved(address indexed validator);
    event ThresholdUpdated(uint256 newThreshold);
    event WithdrawalLimitUpdated(uint256 newLimit);
    event HandlerRegistered(bytes32 indexed schemaHash, address handler);

    // ============ Errors ============

    error MessageAlreadyProcessed();
    error InvalidNonce();
    error InsufficientSignatures();
    error InvalidSignature();
    error DuplicateSignature();
    error MessageTooLarge();
    error DailyLimitExceeded();
    error WithdrawalFailed();
    error CallFailed();
    error NoHandlerForSchema();
    error InvalidValidator();
    error BelowMinimumValidators();

    // ============ Initialization ============

    function initialize(
        address[] memory _validators,
        uint256 _threshold,
        uint256 _withdrawalLimit
    ) public initializer {
        __AccessControl_init();
        __Pausable_init();
        __ReentrancyGuard_init();

        _grantRole(DEFAULT_ADMIN_ROLE, msg.sender);
        _grantRole(ADMIN_ROLE, msg.sender);

        for (uint i = 0; i < _validators.length; i++) {
            _addValidator(_validators[i]);
        }

        validatorThreshold = _threshold;
        dailyWithdrawalLimit = _withdrawalLimit;
        maxMessageSize = 100_000; // 100KB
        minValidators = 3;
        lastWithdrawalReset = block.timestamp;
    }

    // ============ Core Message Processing ============

    /**
     * @notice Process a message from the application's SQL message tables
     * @dev All validation and execution logic is in this contract
     * @param message Message data from validators' replicated SQLite
     * @param signatures Array of validator signatures (v, r, s packed)
     */
    function processMessage(
        Message calldata message,
        bytes[] calldata signatures
    ) external nonReentrant whenNotPaused {
        // Basic validation
        if (processedMessages[message.id]) revert MessageAlreadyProcessed();
        if (message.payload.length > maxMessageSize) revert MessageTooLarge();
        if (message.nonce != schemaNonces[message.schemaHash]) revert InvalidNonce();

        // Verify validator signatures (all logic on-chain)
        _verifySignatures(message, signatures);

        // Mark as processed
        processedMessages[message.id] = true;
        schemaNonces[message.schemaHash]++;

        // Reset daily limit if needed
        _resetDailyLimitIfNeeded();

        // Route to appropriate handler based on message type
        if (message.messageType == MessageType.WITHDRAWAL) {
            _processWithdrawal(message);
        } else if (message.messageType == MessageType.CALL) {
            _processCall(message);
        } else if (message.messageType == MessageType.ORACLE_REQUEST) {
            _processOracleRequest(message);
        } else if (message.messageType == MessageType.ORACLE_RESPONSE) {
            _processOracleResponse(message);
        } else if (message.messageType == MessageType.GOVERNANCE) {
            _processGovernance(message);
        } else if (message.messageType == MessageType.CUSTOM) {
            _processCustom(message);
        }

        emit MessageProcessed(
            message.id,
            message.messageType,
            message.schemaHash,
            msg.sender
        );
    }

    /**
     * @notice Send a message to the SyndDB application (inbound)
     * @dev Emits event that validators listen for and write to inbound_* tables
     * @param targetSchema Hash of the target SQL table schema
     * @param payload ABI-encoded message data
     */
    function sendMessage(
        bytes32 targetSchema,
        bytes calldata payload
    ) external payable nonReentrant whenNotPaused {
        uint256 nonce = currentNonce++;

        emit InboundMessage(
            nonce,
            msg.sender,
            targetSchema,
            payload
        );
    }

    /**
     * @notice Deposit tokens/ETH to the SyndDB application
     * @dev Emits InboundMessage that validators parse and credit in SQL
     */
    function deposit(
        string calldata accountId,
        address token,
        uint256 amount
    ) external payable nonReentrant whenNotPaused {
        if (token == address(0)) {
            // ETH deposit
            require(msg.value == amount, "Invalid ETH amount");
        } else {
            // Token deposit
            IERC20(token).safeTransferFrom(msg.sender, address(this), amount);
        }

        // Schema hash for inbound_deposits table
        bytes32 depositSchema = keccak256("inbound_deposits");

        // Encode deposit data matching SQL table schema
        bytes memory payload = abi.encode(
            accountId,           // account_id
            msg.sender,          // sender_address
            token,               // token_address
            amount,              // amount
            block.number         // block_number
        );

        emit InboundMessage(
            currentNonce++,
            msg.sender,
            depositSchema,
            payload
        );
    }

    // ============ Message Handlers ============

    function _processWithdrawal(Message calldata message) private {
        // Decode according to outbound_withdrawals table schema
        (
            address recipient,
            address token,
            uint256 amount
        ) = abi.decode(message.payload, (address, address, uint256));

        // Check daily limit
        if (dailyWithdrawn + amount > dailyWithdrawalLimit) {
            revert DailyLimitExceeded();
        }
        dailyWithdrawn += amount;

        // Execute withdrawal
        bool success;
        if (token == address(0)) {
            // ETH withdrawal
            (success, ) = recipient.call{value: amount}("");
        } else {
            // Token withdrawal
            try IERC20(token).transfer(recipient, amount) {
                success = true;
            } catch {
                success = false;
            }
        }

        if (!success) revert WithdrawalFailed();

        emit WithdrawalExecuted(message.id, recipient, token, amount);
    }

    function _processCall(Message calldata message) private {
        // Decode according to outbound_calls table schema
        (
            address target,
            bytes memory callData,
            uint256 value
        ) = abi.decode(message.payload, (address, bytes, uint256));

        // Execute call
        (bool success, bytes memory returnData) = target.call{value: value}(callData);

        if (!success) revert CallFailed();

        emit CallExecuted(message.id, target, success, returnData);
    }

    function _processOracleRequest(Message calldata message) private {
        // Forward to registered oracle handler
        address handler = schemaToHandler[message.schemaHash];
        if (handler == address(0)) revert NoHandlerForSchema();

        IOracleReceiver(handler).fulfillOracleRequest(message.payload);
    }

    function _processOracleResponse(Message calldata message) private {
        // Oracle responses are handled by custom logic
        address handler = schemaToHandler[message.schemaHash];
        if (handler == address(0)) revert NoHandlerForSchema();

        IOracleReceiver(handler).receiveOracleResponse(message.payload);
    }

    function _processGovernance(Message calldata message) private {
        // Decode governance action
        (bytes4 selector, bytes memory params) = abi.decode(
            message.payload,
            (bytes4, bytes)
        );

        // Execute governance action (restricted set)
        if (selector == this.updateWithdrawalLimit.selector) {
            uint256 newLimit = abi.decode(params, (uint256));
            _updateWithdrawalLimit(newLimit);
        } else if (selector == this.updateThreshold.selector) {
            uint256 newThreshold = abi.decode(params, (uint256));
            _updateThreshold(newThreshold);
        } else {
            revert("Unknown governance action");
        }
    }

    function _processCustom(Message calldata message) private {
        // Route to custom handler based on schema hash
        address handler = schemaToHandler[message.schemaHash];
        if (handler == address(0)) revert NoHandlerForSchema();

        IMessageHandler(handler).handleMessage(message.payload);
    }

    // ============ Signature Verification (On-Chain) ============

    /**
     * @notice Verify validator signatures on a message
     * @dev All signature verification logic is on-chain in Solidity
     */
    function _verifySignatures(
        Message calldata message,
        bytes[] calldata signatures
    ) private view {
        if (signatures.length < validatorThreshold) {
            revert InsufficientSignatures();
        }

        // Hash the message for signature verification
        bytes32 messageHash = _hashMessage(message);
        bytes32 ethSignedHash = _toEthSignedMessageHash(messageHash);

        // Track unique signers
        address[] memory signers = new address[](signatures.length);

        for (uint i = 0; i < signatures.length; i++) {
            // Recover signer from signature
            address signer = _recoverSigner(ethSignedHash, signatures[i]);

            // Verify signer is a validator
            if (!validators[signer]) revert InvalidValidator();

            // Check for duplicate signatures
            for (uint j = 0; j < i; j++) {
                if (signers[j] == signer) revert DuplicateSignature();
            }

            signers[i] = signer;
        }
    }

    function _hashMessage(Message calldata message) private pure returns (bytes32) {
        return keccak256(abi.encode(
            message.id,
            message.messageType,
            message.schemaHash,
            keccak256(message.payload),
            message.nonce,
            message.timestamp
        ));
    }

    function _toEthSignedMessageHash(bytes32 hash) private pure returns (bytes32) {
        return keccak256(abi.encodePacked("\x19Ethereum Signed Message:\n32", hash));
    }

    function _recoverSigner(
        bytes32 ethSignedHash,
        bytes calldata signature
    ) private pure returns (address) {
        require(signature.length == 65, "Invalid signature length");

        bytes32 r;
        bytes32 s;
        uint8 v;

        assembly {
            r := calldataload(signature.offset)
            s := calldataload(add(signature.offset, 32))
            v := byte(0, calldataload(add(signature.offset, 64)))
        }

        if (v < 27) {
            v += 27;
        }

        require(v == 27 || v == 28, "Invalid signature v value");

        return ecrecover(ethSignedHash, v, r, s);
    }

    // ============ Circuit Breakers ============

    function _resetDailyLimitIfNeeded() private {
        if (block.timestamp >= lastWithdrawalReset + 1 days) {
            dailyWithdrawn = 0;
            lastWithdrawalReset = block.timestamp;
        }
    }

    function pause() external onlyRole(ADMIN_ROLE) {
        _pause();
    }

    function unpause() external onlyRole(ADMIN_ROLE) {
        _unpause();
    }

    // ============ Admin Functions ============

    function addValidator(address validator) external onlyRole(ADMIN_ROLE) {
        _addValidator(validator);
    }

    function _addValidator(address validator) private {
        require(!validators[validator], "Already validator");
        validators[validator] = true;
        validatorList.push(validator);
        _grantRole(VALIDATOR_ROLE, validator);
        emit ValidatorAdded(validator);
    }

    function removeValidator(address validator) external onlyRole(ADMIN_ROLE) {
        require(validatorList.length > minValidators, "Below minimum validators");
        require(validators[validator], "Not a validator");

        validators[validator] = false;
        _revokeRole(VALIDATOR_ROLE, validator);

        // Remove from list
        for (uint i = 0; i < validatorList.length; i++) {
            if (validatorList[i] == validator) {
                validatorList[i] = validatorList[validatorList.length - 1];
                validatorList.pop();
                break;
            }
        }

        emit ValidatorRemoved(validator);
    }

    function updateThreshold(uint256 newThreshold) external onlyRole(ADMIN_ROLE) {
        _updateThreshold(newThreshold);
    }

    function _updateThreshold(uint256 newThreshold) private {
        require(newThreshold > 0, "Threshold must be positive");
        require(newThreshold <= validatorList.length, "Threshold too high");
        validatorThreshold = newThreshold;
        emit ThresholdUpdated(newThreshold);
    }

    function updateWithdrawalLimit(uint256 newLimit) external onlyRole(ADMIN_ROLE) {
        _updateWithdrawalLimit(newLimit);
    }

    function _updateWithdrawalLimit(uint256 newLimit) private {
        dailyWithdrawalLimit = newLimit;
        emit WithdrawalLimitUpdated(newLimit);
    }

    function registerHandler(
        bytes32 schemaHash,
        address handler
    ) external onlyRole(ADMIN_ROLE) {
        schemaToHandler[schemaHash] = handler;
        emit HandlerRegistered(schemaHash, handler);
    }

    function setMaxMessageSize(uint256 newSize) external onlyRole(ADMIN_ROLE) {
        maxMessageSize = newSize;
    }

    // ============ Emergency Functions ============

    function emergencyWithdraw(
        address token,
        address recipient,
        uint256 amount
    ) external onlyRole(DEFAULT_ADMIN_ROLE) whenPaused {
        if (token == address(0)) {
            payable(recipient).transfer(amount);
        } else {
            IERC20(token).safeTransfer(recipient, amount);
        }
    }

    // ============ View Functions ============

    function getValidators() external view returns (address[] memory) {
        return validatorList;
    }

    function getValidatorCount() external view returns (uint256) {
        return validatorList.length;
    }

    function isMessageProcessed(uint256 messageId) external view returns (bool) {
        return processedMessages[messageId];
    }

    function getCurrentNonce(bytes32 schemaHash) external view returns (uint256) {
        return schemaNonces[schemaHash];
    }

    // ============ Receive ETH ============

    receive() external payable {}
}
```

### Message Handler Interfaces

```solidity
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

interface IMessageHandler {
    function handleMessage(bytes calldata payload) external;
}

interface IOracleReceiver {
    function fulfillOracleRequest(bytes calldata requestData) external;
    function receiveOracleResponse(bytes calldata responseData) external;
}
```

### Example: Custom Swap Handler

```solidity
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import "./interfaces/IMessageHandler.sol";
import "@uniswap/v3-periphery/contracts/interfaces/ISwapRouter.sol";

contract SwapHandler is IMessageHandler {
    ISwapRouter public immutable swapRouter;
    address public immutable bridge;

    modifier onlyBridge() {
        require(msg.sender == bridge, "Only bridge");
        _;
    }

    constructor(address _swapRouter, address _bridge) {
        swapRouter = ISwapRouter(_swapRouter);
        bridge = _bridge;
    }

    function handleMessage(bytes calldata payload) external override onlyBridge {
        // Decode swap data from SQL table
        (
            address tokenIn,
            address tokenOut,
            uint256 amountIn,
            uint256 minAmountOut,
            address recipient
        ) = abi.decode(payload, (address, address, uint256, uint256, address));

        // Approve router
        IERC20(tokenIn).approve(address(swapRouter), amountIn);

        // Execute swap
        ISwapRouter.ExactInputSingleParams memory params = ISwapRouter
            .ExactInputSingleParams({
                tokenIn: tokenIn,
                tokenOut: tokenOut,
                fee: 3000,
                recipient: recipient,
                deadline: block.timestamp,
                amountIn: amountIn,
                amountOutMinimum: minAmountOut,
                sqrtPriceLimitX96: 0
            });

        swapRouter.exactInputSingle(params);
    }
}
```

## Message Table Schemas

Applications define message tables in SQL that map to contract ABIs:

### Standard Withdrawal Table

```sql
-- Table schema defines the ABI encoding
CREATE TABLE outbound_withdrawals (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id TEXT NOT NULL,              -- Internal account
    recipient_address TEXT NOT NULL,       -- External address (0x...)
    token_address TEXT NOT NULL,           -- Token or 0x0 for ETH
    amount TEXT NOT NULL,                  -- Wei as string
    status TEXT DEFAULT 'pending',         -- pending|submitted|confirmed|failed
    validator_signatures TEXT,              -- Reserved for coordinator
    tx_hash TEXT,                          -- Settlement tx hash
    created_at INTEGER DEFAULT (unixepoch()),
    processed_at INTEGER,
    INDEX idx_status (status),
    CHECK (status IN ('pending', 'submitted', 'confirmed', 'failed'))
);

-- Application just does:
INSERT INTO outbound_withdrawals (account_id, recipient_address, token_address, amount)
VALUES ('alice', '0x742d35Cc6634C0532925a3b844Bc9e7595f0bEb', '0x0', '1000000000000000000');

-- Validators detect this, sign it, submit to Bridge.sol
-- Bridge.sol calls: _processWithdrawal() which does the transfer
```

### Inbound Deposits

```sql
CREATE TABLE inbound_deposits (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    source_tx_hash TEXT UNIQUE NOT NULL,   -- Ethereum tx hash
    sender_address TEXT NOT NULL,          -- Sender on Ethereum
    account_id TEXT NOT NULL,              -- Target account in app
    token_address TEXT NOT NULL,
    amount TEXT NOT NULL,
    block_number INTEGER NOT NULL,
    status TEXT DEFAULT 'pending',
    created_at INTEGER DEFAULT (unixepoch()),
    credited_at INTEGER,
    INDEX idx_account_status (account_id, status)
);

-- Validators listen for Bridge.deposit() events
-- Parse event data and INSERT into this table
-- Application queries this table to credit accounts
```

### Generic Message Calls

```sql
CREATE TABLE outbound_calls (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    target_contract TEXT NOT NULL,         -- Contract to call
    function_selector TEXT NOT NULL,       -- bytes4 selector
    call_data BLOB NOT NULL,               -- ABI-encoded params
    value TEXT DEFAULT '0',                -- ETH to send
    status TEXT DEFAULT 'pending',
    tx_hash TEXT,
    created_at INTEGER DEFAULT (unixepoch())
);

-- Application can trigger arbitrary contract calls
-- Bridge.sol executes via _processCall()
```

### Schema Hash Calculation

```javascript
// Schema hash ties SQL table to contract ABI
function calculateSchemaHash(tableName, columns) {
  const schemaString = JSON.stringify({
    table: tableName,
    columns: columns.map(c => ({ name: c.name, type: c.abiType }))
  });
  return ethers.utils.keccak256(ethers.utils.toUtf8Bytes(schemaString));
}

// Example:
const withdrawalSchema = calculateSchemaHash("outbound_withdrawals", [
  { name: "recipient_address", abiType: "address" },
  { name: "token_address", abiType: "address" },
  { name: "amount", abiType: "uint256" }
]);
```

## Minimal Validator Integration (Rust)

Validators are read replicas (see PLAN_REPLICA.md) with additional bridge message processing. The Rust code is minimal - just monitoring tables and calling Bridge.sol:

### Message Processor Module (Part of Replica)

```rust
// This lives in synddb-replica/src/bridge.rs
use alloy::prelude::*;
use rusqlite::Connection;

pub struct BridgeProcessor {
    db: Arc<Connection>,
    bridge: BridgeContract,
    signer: PrivateKeySigner,
    monitored_tables: Vec<TableConfig>,
}

pub struct TableConfig {
    pub name: String,
    pub schema_hash: B256,
    pub message_type: u8,  // Maps to MessageType enum
    pub columns: Vec<String>,
}

impl BridgeProcessor {
    pub async fn process_outbound_messages(&self) -> Result<()> {
        for table in &self.monitored_tables {
            // Simple SQL query for pending messages
            let sql = format!(
                "SELECT * FROM {} WHERE status = 'pending' ORDER BY id LIMIT 10",
                table.name
            );

            let mut stmt = self.db.prepare(&sql)?;
            let rows = stmt.query_map([], |row| {
                // Extract row data into Message struct
                Ok(self.build_message(row, table)?)
            })?;

            for message in rows {
                let msg = message?;

                // Sign the message
                let signature = self.sign_message(&msg)?;

                // Coordinate with other validators (simple HTTP)
                let all_signatures = self.gather_signatures(&msg, signature).await?;

                // Submit to Bridge.sol
                if all_signatures.len() >= self.threshold() {
                    let tx = self.bridge
                        .processMessage(msg.clone(), all_signatures)
                        .send()
                        .await?;

                    // Update status in SQL
                    self.db.execute(
                        &format!("UPDATE {} SET status = 'submitted', tx_hash = ?1 WHERE id = ?2", table.name),
                        params![tx.tx_hash().to_string(), msg.id]
                    )?;
                }
            }
        }
        Ok(())
    }

    fn sign_message(&self, message: &Message) -> Result<Bytes> {
        // Hash the message (same as Solidity _hashMessage)
        let message_hash = keccak256(&abi::encode(&[
            message.id.to_token(),
            message.message_type.to_token(),
            message.schema_hash.to_token(),
            keccak256(&message.payload).to_token(),
            message.nonce.to_token(),
            message.timestamp.to_token(),
        ]));

        // Sign with Ethereum signed message prefix
        let signature = self.signer.sign_message(&message_hash).await?;
        Ok(signature.as_bytes().into())
    }

    async fn gather_signatures(
        &self,
        message: &Message,
        own_sig: Bytes
    ) -> Result<Vec<Bytes>> {
        // Simple HTTP-based coordination (can be enhanced)
        let mut signatures = vec![own_sig];

        for validator in &self.other_validators {
            let client = reqwest::Client::new();
            let resp: SignatureResponse = client
                .post(&format!("{}/sign", validator.url))
                .json(&message)
                .send()
                .await?
                .json()
                .await?;

            signatures.push(resp.signature);

            if signatures.len() >= self.threshold() {
                break;
            }
        }

        Ok(signatures)
    }

    pub async fn process_inbound_messages(&self) -> Result<()> {
        // Listen for Bridge.InboundMessage events
        let filter = self.bridge
            .InboundMessage_filter()
            .from_block(self.last_block);

        let logs = filter.query().await?;

        for log in logs {
            // Decode event data
            let (nonce, sender, schema_hash, payload) = (
                log.nonce,
                log.sender,
                log.targetSchema,
                log.payload,
            );

            // Find matching table
            let table = self.find_inbound_table(schema_hash)?;

            // Decode payload and insert into SQL
            self.insert_inbound(&table, payload).await?;
        }

        Ok(())
    }

    fn insert_inbound(&self, table: &str, payload: Bytes) -> Result<()> {
        // Decode based on table schema and insert
        // This is simple SQL INSERT - no complex logic
        match table {
            "inbound_deposits" => {
                let (account_id, sender, token, amount, block) =
                    abi::decode(&["string", "address", "address", "uint256", "uint256"], &payload)?;

                self.db.execute(
                    "INSERT INTO inbound_deposits (account_id, sender_address, token_address, amount, block_number, status)
                     VALUES (?1, ?2, ?3, ?4, ?5, 'pending')",
                    params![account_id, sender, token, amount.to_string(), block]
                )?;
            }
            _ => {
                // Custom table handling
            }
        }
        Ok(())
    }
}
```

**Key Point**: The Rust code is simple - just:
1. Query SQL for pending messages
2. Sign messages locally
3. Coordinate via HTTP to gather signatures
4. Submit to Bridge.sol
5. Listen for events and write to SQL

All complex logic (signature verification, multi-sig, execution) is in Solidity.

## Testing

### Solidity Tests (Foundry)

```solidity
// test/Bridge.t.sol
pragma solidity ^0.8.20;

import "forge-std/Test.sol";
import "../src/Bridge.sol";

contract BridgeTest is Test {
    Bridge public bridge;
    address[] public validators;
    uint256[] public validatorKeys;

    function setUp() public {
        // Setup validators
        for (uint i = 0; i < 3; i++) {
            uint256 key = uint256(keccak256(abi.encode(i)));
            validators.push(vm.addr(key));
            validatorKeys.push(key);
        }

        // Deploy bridge
        bridge = new Bridge();
        bridge.initialize(
            validators,
            2,  // 2-of-3 multisig
            1000 ether
        );

        // Fund bridge
        vm.deal(address(bridge), 100 ether);
    }

    function testWithdrawal() public {
        // Create message
        Bridge.Message memory msg = Bridge.Message({
            id: 1,
            messageType: Bridge.MessageType.WITHDRAWAL,
            schemaHash: keccak256("outbound_withdrawals"),
            payload: abi.encode(address(this), address(0), 1 ether),
            nonce: 0,
            timestamp: block.timestamp
        });

        // Get signatures from validators
        bytes[] memory sigs = new bytes[](2);
        sigs[0] = _signMessage(msg, validatorKeys[0]);
        sigs[1] = _signMessage(msg, validatorKeys[1]);

        // Process message
        uint256 balanceBefore = address(this).balance;
        bridge.processMessage(msg, sigs);

        assertEq(address(this).balance - balanceBefore, 1 ether);
        assertTrue(bridge.processedMessages(1));
    }

    function testInsufficientSignatures() public {
        Bridge.Message memory msg = _createTestMessage();
        bytes[] memory sigs = new bytes[](1);  // Only 1 signature
        sigs[0] = _signMessage(msg, validatorKeys[0]);

        vm.expectRevert(Bridge.InsufficientSignatures.selector);
        bridge.processMessage(msg, sigs);
    }

    function testDailyLimit() public {
        // Process withdrawals up to limit
        // ...

        // Next withdrawal should fail
        vm.expectRevert(Bridge.DailyLimitExceeded.selector);
        bridge.processMessage(largeWithdrawal, sigs);
    }

    function _signMessage(
        Bridge.Message memory msg,
        uint256 privateKey
    ) internal pure returns (bytes memory) {
        bytes32 hash = _hashMessage(msg);
        bytes32 ethHash = keccak256(abi.encodePacked(
            "\x19Ethereum Signed Message:\n32",
            hash
        ));
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(privateKey, ethHash);
        return abi.encodePacked(r, s, v);
    }

    function _hashMessage(Bridge.Message memory msg) internal pure returns (bytes32) {
        return keccak256(abi.encode(
            msg.id,
            msg.messageType,
            msg.schemaHash,
            keccak256(msg.payload),
            msg.nonce,
            msg.timestamp
        ));
    }

    receive() external payable {}
}
```

### Integration Test (Rust + Solidity)

```rust
#[tokio::test]
async fn test_end_to_end_withdrawal() {
    // Deploy bridge contract
    let bridge = deploy_bridge_contract().await;

    // Setup test database with message table
    let db = setup_test_db().await;
    db.execute(
        "CREATE TABLE outbound_withdrawals (
            id INTEGER PRIMARY KEY,
            account_id TEXT,
            recipient_address TEXT,
            token_address TEXT,
            amount TEXT,
            status TEXT DEFAULT 'pending'
        )",
        []
    ).unwrap();

    // Insert withdrawal message
    db.execute(
        "INSERT INTO outbound_withdrawals (account_id, recipient_address, token_address, amount)
         VALUES ('alice', '0x70997970C51812dc3A010C7d01b50e0d17dc79C8', '0x0', '1000000000000000000')",
        []
    ).unwrap();

    // Create processor
    let processor = BridgeProcessor::new(db, bridge);

    // Process messages
    processor.process_outbound_messages().await.unwrap();

    // Verify on-chain
    let processed = bridge.processedMessages(U256::from(1)).call().await.unwrap();
    assert!(processed._0);
}
```

## Deployment

### Deploy Script (Foundry)

```solidity
// script/DeployBridge.s.sol
pragma solidity ^0.8.20;

import "forge-std/Script.sol";
import "../src/Bridge.sol";

contract DeployBridge is Script {
    function run() external {
        uint256 deployerKey = vm.envUint("PRIVATE_KEY");

        address[] memory validators = new address[](3);
        validators[0] = vm.envAddress("VALIDATOR_1");
        validators[1] = vm.envAddress("VALIDATOR_2");
        validators[2] = vm.envAddress("VALIDATOR_3");

        vm.startBroadcast(deployerKey);

        Bridge bridge = new Bridge();
        bridge.initialize(
            validators,
            2,  // threshold
            1000 ether  // daily limit
        );

        vm.stopBroadcast();

        console.log("Bridge deployed at:", address(bridge));
    }
}
```

### Validator Config

```yaml
# validator-config.yaml
bridge:
  contract_address: "0x..."
  chain_id: 1
  rpc_url: "https://eth-mainnet.alchemyapi.io/v2/..."

  # Message tables to monitor
  outbound_tables:
    - name: "outbound_withdrawals"
      schema_hash: "0x1234..."
      message_type: 0  # WITHDRAWAL
      poll_interval_secs: 5

    - name: "outbound_calls"
      schema_hash: "0x5678..."
      message_type: 2  # CALL
      poll_interval_secs: 10

  # Inbound tables to populate
  inbound_tables:
    - name: "inbound_deposits"
      schema_hash: "0xabcd..."
      event: "InboundMessage"

  # Validator coordination
  other_validators:
    - url: "https://validator2.example.com"
    - url: "https://validator3.example.com"

  threshold: 2

  # Signer
  private_key: "${VALIDATOR_PRIVATE_KEY}"
```

## Summary

The Bridge is **primarily a Solidity smart contract**. All complex logic lives on-chain:
- ✅ Multi-sig verification
- ✅ Message processing and routing
- ✅ Withdrawal execution
- ✅ Circuit breakers
- ✅ Nonce management

Validators are simple:
- ✅ Monitor SQL tables for pending messages
- ✅ Sign messages locally
- ✅ Coordinate signatures via HTTP
- ✅ Submit to Bridge.sol
- ✅ Listen for events and write to inbound tables

This keeps the architecture clean and makes the bridge behavior fully transparent and verifiable on-chain.
