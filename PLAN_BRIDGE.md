# PLAN_BRIDGE.md - Generic Message Passing System

## Overview

The SyndDB bridge is a generalized message passing system that enables cross-chain communication through SQLite tables. Unlike traditional bridges that hardcode specific operations, this system allows applications to define their own message schemas in SQL tables, which automatically map to smart contract ABIs. Validators monitor these tables and process messages according to the defined schemas.

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                     Application (Any Language)                  │
│  ┌─────────────────────────────────────────────────────────┐   │
│  │  Writes to Message Tables (outbound_*, inbound_*)       │   │
│  │  - outbound_withdrawals                                  │   │
│  │  - outbound_messages                                     │   │
│  │  - inbound_deposits                                      │   │
│  │  - custom_message_tables                                 │   │
│  └─────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────┘
                              ↓
┌─────────────────────────────────────────────────────────────────┐
│                    Validator Network (TEEs)                     │
│  ┌─────────────────────────────────────────────────────────┐   │
│  │  1. Monitor message tables for new entries               │   │
│  │  2. Validate message according to schema                 │   │
│  │  3. Gather multi-sig from validator set                  │   │
│  │  4. Submit to Bridge.sol on settlement chain             │   │
│  └─────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────┘
                              ↓
┌─────────────────────────────────────────────────────────────────┐
│                    Settlement Chain (Ethereum)                  │
│  ┌─────────────────────────────────────────────────────────┐   │
│  │                      Bridge.sol                          │   │
│  │  - Processes messages with validator signatures          │   │
│  │  - Executes withdrawals, deposits, calls                 │   │
│  │  - Emits events for inbound message processing           │   │
│  └─────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────┘
                              ↓
┌─────────────────────────────────────────────────────────────────┐
│                 Validator Inbound Processing                    │
│  ┌─────────────────────────────────────────────────────────┐   │
│  │  1. Monitor Bridge.sol events                            │   │
│  │  2. Parse inbound messages                               │   │
│  │  3. Write to application's inbound_* tables              │   │
│  │  4. Application processes inbound messages               │   │
│  └─────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────┘
```

## Smart Contract Architecture

### Core Bridge Contract

```solidity
// contracts/Bridge.sol
pragma solidity ^0.8.19;

import "@openzeppelin/contracts-upgradeable/proxy/utils/Initializable.sol";
import "@openzeppelin/contracts-upgradeable/access/AccessControlUpgradeable.sol";
import "@openzeppelin/contracts-upgradeable/security/PausableUpgradeable.sol";
import "@openzeppelin/contracts-upgradeable/security/ReentrancyGuardUpgradeable.sol";

contract Bridge is 
    Initializable,
    AccessControlUpgradeable,
    PausableUpgradeable,
    ReentrancyGuardUpgradeable 
{
    bytes32 public constant VALIDATOR_ROLE = keccak256("VALIDATOR_ROLE");
    bytes32 public constant ADMIN_ROLE = keccak256("ADMIN_ROLE");
    
    // Message types
    enum MessageType {
        WITHDRAWAL,
        DEPOSIT,
        CALL,
        ORACLE_REQUEST,
        ORACLE_RESPONSE,
        GOVERNANCE,
        CUSTOM
    }
    
    struct Message {
        uint256 id;
        MessageType messageType;
        bytes32 schemaHash;  // Hash of table schema for verification
        bytes payload;        // ABI-encoded data from table row
        uint256 nonce;
        uint256 timestamp;
    }
    
    struct ValidatorSignature {
        address validator;
        bytes signature;
        bytes attestation;  // TEE attestation
    }
    
    // State
    mapping(uint256 => bool) public processedMessages;
    mapping(address => bool) public validators;
    mapping(bytes32 => address) public schemaToHandler;  // Custom handlers
    
    uint256 public validatorThreshold;
    uint256 public currentNonce;
    
    // Circuit breakers
    uint256 public dailyWithdrawalLimit;
    uint256 public dailyWithdrawn;
    uint256 public lastWithdrawalReset;
    uint256 public maxMessageSize;
    
    // Events
    event MessageProcessed(
        uint256 indexed messageId,
        MessageType messageType,
        bytes32 schemaHash,
        bytes payload,
        address indexed processor
    );
    
    event InboundMessage(
        uint256 indexed nonce,
        address indexed sender,
        bytes32 targetSchema,
        bytes payload
    );
    
    event ValidatorAdded(address indexed validator, bytes attestation);
    event ValidatorRemoved(address indexed validator);
    event WithdrawalLimitUpdated(uint256 newLimit);
    
    function initialize(
        address[] memory _validators,
        uint256 _threshold,
        uint256 _withdrawalLimit
    ) public initializer {
        __AccessControl_init();
        __Pausable_init();
        __ReentrancyGuard_init();
        
        _setupRole(DEFAULT_ADMIN_ROLE, msg.sender);
        _setupRole(ADMIN_ROLE, msg.sender);
        
        for (uint i = 0; i < _validators.length; i++) {
            validators[_validators[i]] = true;
            _setupRole(VALIDATOR_ROLE, _validators[i]);
        }
        
        validatorThreshold = _threshold;
        dailyWithdrawalLimit = _withdrawalLimit;
        maxMessageSize = 10000; // 10KB default
        lastWithdrawalReset = block.timestamp;
    }
    
    /// @notice Process a message from the application's message tables
    /// @param message The message data from SQL table
    /// @param signatures Validator signatures approving this message
    function processMessage(
        Message calldata message,
        ValidatorSignature[] calldata signatures
    ) external nonReentrant whenNotPaused {
        // Check message not already processed
        require(!processedMessages[message.id], "Message already processed");
        require(message.payload.length <= maxMessageSize, "Message too large");
        
        // Verify signatures
        require(signatures.length >= validatorThreshold, "Insufficient signatures");
        _verifySignatures(message, signatures);
        
        // Mark as processed
        processedMessages[message.id] = true;
        
        // Reset daily limit if needed
        if (block.timestamp >= lastWithdrawalReset + 1 days) {
            dailyWithdrawn = 0;
            lastWithdrawalReset = block.timestamp;
        }
        
        // Process based on message type
        if (message.messageType == MessageType.WITHDRAWAL) {
            _processWithdrawal(message);
        } else if (message.messageType == MessageType.CALL) {
            _processCall(message);
        } else if (message.messageType == MessageType.ORACLE_REQUEST) {
            _processOracleRequest(message);
        } else if (message.messageType == MessageType.GOVERNANCE) {
            _processGovernance(message);
        } else if (message.messageType == MessageType.CUSTOM) {
            _processCustom(message);
        }
        
        emit MessageProcessed(
            message.id,
            message.messageType,
            message.schemaHash,
            message.payload,
            msg.sender
        );
    }
    
    /// @notice Send a message to the SyndDB application
    /// @param targetSchema The schema hash of the target inbound table
    /// @param payload The message data to send
    function sendMessage(
        bytes32 targetSchema,
        bytes calldata payload
    ) external payable nonReentrant {
        uint256 nonce = currentNonce++;
        
        emit InboundMessage(
            nonce,
            msg.sender,
            targetSchema,
            payload
        );
    }
    
    function _processWithdrawal(Message memory message) private {
        // Decode withdrawal data based on table schema
        (
            address recipient,
            address token,
            uint256 amount
        ) = abi.decode(message.payload, (address, address, uint256));
        
        // Check daily limit
        require(dailyWithdrawn + amount <= dailyWithdrawalLimit, "Daily limit exceeded");
        dailyWithdrawn += amount;
        
        // Execute withdrawal
        if (token == address(0)) {
            // ETH withdrawal
            (bool success, ) = recipient.call{value: amount}("");
            require(success, "ETH transfer failed");
        } else {
            // Token withdrawal
            IERC20(token).safeTransfer(recipient, amount);
        }
    }
    
    function _processCall(Message memory message) private {
        (
            address target,
            bytes memory callData,
            uint256 value
        ) = abi.decode(message.payload, (address, bytes, uint256));
        
        // Execute external call
        (bool success, bytes memory result) = target.call{value: value}(callData);
        require(success, "External call failed");
    }
    
    function _processOracleRequest(Message memory message) private {
        // Forward to oracle contract
        IOracleReceiver oracle = IOracleReceiver(schemaToHandler[message.schemaHash]);
        oracle.fulfillOracleRequest(message.payload);
    }
    
    function _processGovernance(Message memory message) private {
        (
            bytes4 selector,
            bytes memory params
        ) = abi.decode(message.payload, (bytes4, bytes));
        
        // Execute governance action
        if (selector == this.updateWithdrawalLimit.selector) {
            uint256 newLimit = abi.decode(params, (uint256));
            _updateWithdrawalLimit(newLimit);
        } else if (selector == this.addValidator.selector) {
            (address validator, bytes memory attestation) = abi.decode(params, (address, bytes));
            _addValidator(validator, attestation);
        }
    }
    
    function _processCustom(Message memory message) private {
        // Route to custom handler based on schema
        address handler = schemaToHandler[message.schemaHash];
        require(handler != address(0), "No handler for schema");
        
        IMessageHandler(handler).handleMessage(message.payload);
    }
    
    function _verifySignatures(
        Message memory message,
        ValidatorSignature[] memory signatures
    ) private view {
        bytes32 messageHash = keccak256(abi.encode(message));
        
        address[] memory signers = new address[](signatures.length);
        
        for (uint i = 0; i < signatures.length; i++) {
            // Recover signer
            address signer = ECDSA.recover(messageHash, signatures[i].signature);
            
            // Check validator status
            require(validators[signer], "Invalid validator");
            
            // Check for duplicates
            for (uint j = 0; j < i; j++) {
                require(signers[j] != signer, "Duplicate signature");
            }
            
            signers[i] = signer;
            
            // Verify TEE attestation if provided
            if (signatures[i].attestation.length > 0) {
                _verifyAttestation(signer, signatures[i].attestation);
            }
        }
    }
    
    function _verifyAttestation(address validator, bytes memory attestation) private view {
        // Verify TEE attestation (simplified - real implementation would verify quote)
        IAttestationVerifier verifier = IAttestationVerifier(schemaToHandler[keccak256("attestation")]);
        require(verifier.verifyAttestation(validator, attestation), "Invalid attestation");
    }
    
    // Admin functions
    function updateWithdrawalLimit(uint256 newLimit) external onlyRole(ADMIN_ROLE) {
        _updateWithdrawalLimit(newLimit);
    }
    
    function _updateWithdrawalLimit(uint256 newLimit) private {
        dailyWithdrawalLimit = newLimit;
        emit WithdrawalLimitUpdated(newLimit);
    }
    
    function addValidator(address validator, bytes calldata attestation) external onlyRole(ADMIN_ROLE) {
        _addValidator(validator, attestation);
    }
    
    function _addValidator(address validator, bytes memory attestation) private {
        validators[validator] = true;
        _setupRole(VALIDATOR_ROLE, validator);
        emit ValidatorAdded(validator, attestation);
    }
    
    function removeValidator(address validator) external onlyRole(ADMIN_ROLE) {
        validators[validator] = false;
        _revokeRole(VALIDATOR_ROLE, validator);
        emit ValidatorRemoved(validator);
    }
    
    function registerHandler(bytes32 schemaHash, address handler) external onlyRole(ADMIN_ROLE) {
        schemaToHandler[schemaHash] = handler;
    }
    
    function pause() external onlyRole(ADMIN_ROLE) {
        _pause();
    }
    
    function unpause() external onlyRole(ADMIN_ROLE) {
        _unpause();
    }
    
    // Emergency functions
    function emergencyWithdraw(
        address token,
        address recipient,
        uint256 amount
    ) external onlyRole(DEFAULT_ADMIN_ROLE) {
        if (token == address(0)) {
            payable(recipient).transfer(amount);
        } else {
            IERC20(token).safeTransfer(recipient, amount);
        }
    }
    
    receive() external payable {}
}
```

### Message Handler Interface

```solidity
// contracts/interfaces/IMessageHandler.sol
pragma solidity ^0.8.19;

interface IMessageHandler {
    function handleMessage(bytes calldata payload) external;
}

interface IOracleReceiver {
    function fulfillOracleRequest(bytes calldata response) external;
}

interface IAttestationVerifier {
    function verifyAttestation(
        address validator,
        bytes calldata attestation
    ) external view returns (bool);
}
```

### Example Custom Handler

```solidity
// contracts/handlers/CrossChainSwapHandler.sol
pragma solidity ^0.8.19;

contract CrossChainSwapHandler is IMessageHandler {
    struct SwapMessage {
        address tokenIn;
        address tokenOut;
        uint256 amountIn;
        uint256 minAmountOut;
        address recipient;
        bytes swapData;
    }
    
    function handleMessage(bytes calldata payload) external override {
        SwapMessage memory swap = abi.decode(payload, (SwapMessage));
        
        // Execute swap logic
        _executeSwap(swap);
    }
    
    function _executeSwap(SwapMessage memory swap) private {
        // Integration with DEX aggregator
        // ...
    }
}
```

## Message Table Schemas

Applications define message tables that map to contract ABIs:

### Standard Message Tables

```sql
-- Outbound withdrawal messages
CREATE TABLE outbound_withdrawals (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id TEXT NOT NULL,              -- Internal account identifier
    recipient_address TEXT NOT NULL,       -- Ethereum address (0x...)
    token_address TEXT NOT NULL,           -- Token contract or 'ETH'
    amount TEXT NOT NULL,                  -- Wei amount as string
    status TEXT DEFAULT 'pending',         -- pending, submitted, confirmed, failed
    validator_signatures TEXT,              -- JSON array of signatures
    tx_hash TEXT,                          -- Settlement transaction hash
    created_at INTEGER DEFAULT (unixepoch()),
    processed_at INTEGER,
    INDEX idx_status (status),
    INDEX idx_created (created_at)
);

-- Inbound deposit messages
CREATE TABLE inbound_deposits (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    source_tx_hash TEXT UNIQUE NOT NULL,   -- Ethereum transaction hash
    sender_address TEXT NOT NULL,          -- Source Ethereum address
    account_id TEXT NOT NULL,              -- Target account in system
    token_address TEXT NOT NULL,           -- Token contract or 'ETH'
    amount TEXT NOT NULL,                  -- Wei amount as string
    block_number INTEGER NOT NULL,         -- Ethereum block number
    status TEXT DEFAULT 'pending',         -- pending, credited, failed
    created_at INTEGER DEFAULT (unixepoch()),
    credited_at INTEGER,
    INDEX idx_account (account_id),
    INDEX idx_status (status)
);

-- Generic outbound messages
CREATE TABLE outbound_messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    message_type TEXT NOT NULL,            -- CALL, ORACLE_REQUEST, GOVERNANCE, CUSTOM
    target_address TEXT NOT NULL,          -- Target contract address
    function_signature TEXT NOT NULL,      -- Function selector (0x...)
    parameters BLOB NOT NULL,              -- ABI-encoded parameters
    value TEXT DEFAULT '0',                -- ETH value to send
    gas_limit INTEGER DEFAULT 500000,      -- Gas limit for call
    status TEXT DEFAULT 'pending',
    validator_signatures TEXT,
    tx_hash TEXT,
    created_at INTEGER DEFAULT (unixepoch()),
    processed_at INTEGER,
    INDEX idx_type_status (message_type, status)
);

-- Cross-chain call messages
CREATE TABLE outbound_calls (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    target_chain TEXT NOT NULL,            -- ethereum, polygon, arbitrum, etc.
    target_contract TEXT NOT NULL,         -- Contract address on target chain
    method_name TEXT NOT NULL,             -- Human-readable method name
    method_signature TEXT NOT NULL,        -- Method signature bytes4
    parameters JSON NOT NULL,              -- JSON parameters (converted to ABI)
    value TEXT DEFAULT '0',                -- Native token value
    status TEXT DEFAULT 'pending',
    response JSON,                         -- Response from target chain
    created_at INTEGER DEFAULT (unixepoch()),
    executed_at INTEGER,
    INDEX idx_chain_status (target_chain, status)
);

-- Oracle request/response tables
CREATE TABLE oracle_requests (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    request_type TEXT NOT NULL,            -- PRICE, RANDOM, WEATHER, CUSTOM
    request_params JSON NOT NULL,          -- Request parameters
    callback_table TEXT,                   -- Table to write response to
    callback_id TEXT,                      -- ID in callback table
    status TEXT DEFAULT 'pending',
    response JSON,
    created_at INTEGER DEFAULT (unixepoch()),
    fulfilled_at INTEGER
);

CREATE TABLE oracle_responses (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    request_id INTEGER NOT NULL,
    response_data JSON NOT NULL,
    attestation TEXT,                      -- Oracle attestation/signature
    created_at INTEGER DEFAULT (unixepoch()),
    FOREIGN KEY (request_id) REFERENCES oracle_requests(id)
);
```

### Custom Application Tables

Applications can define custom message tables:

```sql
-- Example: NFT bridging
CREATE TABLE outbound_nft_transfers (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    nft_contract TEXT NOT NULL,
    token_id TEXT NOT NULL,
    from_account TEXT NOT NULL,
    to_address TEXT NOT NULL,
    metadata JSON,
    status TEXT DEFAULT 'pending',
    created_at INTEGER DEFAULT (unixepoch())
);

-- Example: Governance proposals
CREATE TABLE governance_proposals (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    proposal_type TEXT NOT NULL,           -- PARAMETER, UPGRADE, CUSTOM
    title TEXT NOT NULL,
    description TEXT,
    actions JSON NOT NULL,                 -- Array of actions to execute
    voting_start INTEGER,
    voting_end INTEGER,
    for_votes INTEGER DEFAULT 0,
    against_votes INTEGER DEFAULT 0,
    status TEXT DEFAULT 'pending',
    execution_tx TEXT,
    created_at INTEGER DEFAULT (unixepoch())
);
```

## Validator Message Processing

### Message Detection and Processing

```rust
// validator/src/message_processor.rs
use alloy::prelude::*;
use rusqlite::Connection;

pub struct MessageProcessor {
    db: Arc<Connection>,
    bridge_contract: BridgeContract,
    validator_key: SigningKey,
    tee_attestor: Option<TeeAttestor>,
    monitored_tables: Vec<TableSchema>,
}

#[derive(Clone)]
pub struct TableSchema {
    pub name: String,
    pub message_type: MessageType,
    pub columns: Vec<Column>,
    pub abi_encoder: Box<dyn AbiEncoder>,
}

pub struct Column {
    pub name: String,
    pub sql_type: String,
    pub abi_type: String,
    pub required: bool,
}

impl MessageProcessor {
    pub async fn start(mut self) -> Result<()> {
        let mut interval = tokio::time::interval(Duration::from_secs(10));
        
        loop {
            interval.tick().await;
            self.process_pending_messages().await?;
            self.process_inbound_messages().await?;
        }
    }
    
    async fn process_pending_messages(&mut self) -> Result<()> {
        for schema in &self.monitored_tables {
            let messages = self.fetch_pending_messages(&schema).await?;
            
            for message in messages {
                // Validate message
                if !self.validate_message(&message, &schema)? {
                    self.mark_failed(message.id, &schema.name).await?;
                    continue;
                }
                
                // Convert to contract message
                let contract_message = self.encode_message(&message, &schema)?;
                
                // Sign message
                let signature = self.sign_message(&contract_message)?;
                
                // Generate TEE attestation if available
                let attestation = if let Some(attestor) = &self.tee_attestor {
                    Some(attestor.attest(&contract_message)?)
                } else {
                    None
                };
                
                // Broadcast to other validators
                let signatures = self.gather_signatures(
                    &contract_message,
                    signature,
                    attestation
                ).await?;
                
                // Submit to bridge if threshold met
                if signatures.len() >= self.threshold() {
                    let tx_hash = self.submit_to_bridge(
                        contract_message,
                        signatures
                    ).await?;
                    
                    self.mark_processed(
                        message.id,
                        &schema.name,
                        &tx_hash
                    ).await?;
                }
            }
        }
        
        Ok(())
    }
    
    async fn fetch_pending_messages(&self, schema: &TableSchema) -> Result<Vec<Message>> {
        let sql = format!(
            "SELECT * FROM {} WHERE status = 'pending' ORDER BY id LIMIT 100",
            schema.name
        );
        
        let mut stmt = self.db.prepare(&sql)?;
        let message_iter = stmt.query_map([], |row| {
            Ok(Message {
                id: row.get("id")?,
                data: self.extract_row_data(row, &schema.columns)?,
            })
        })?;
        
        let mut messages = Vec::new();
        for message in message_iter {
            messages.push(message?);
        }
        
        Ok(messages)
    }
    
    fn encode_message(&self, message: &Message, schema: &TableSchema) -> Result<ContractMessage> {
        let payload = schema.abi_encoder.encode(&message.data)?;
        let schema_hash = keccak256(schema.name.as_bytes());
        
        Ok(ContractMessage {
            id: message.id as u256,
            message_type: schema.message_type,
            schema_hash,
            payload,
            nonce: self.next_nonce(),
            timestamp: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
        })
    }
    
    async fn submit_to_bridge(
        &self,
        message: ContractMessage,
        signatures: Vec<ValidatorSignature>
    ) -> Result<TxHash> {
        let tx = self.bridge_contract
            .process_message(message, signatures)
            .from(self.validator_key.address())
            .gas_price(self.get_gas_price().await?)
            .send()
            .await?;
            
        info!("Message submitted to bridge: {:?}", tx.hash());
        Ok(tx.hash())
    }
    
    async fn process_inbound_messages(&mut self) -> Result<()> {
        // Query bridge contract for new InboundMessage events
        let filter = self.bridge_contract
            .inbound_message_filter()
            .from_block(self.last_processed_block);
            
        let logs = filter.query().await?;
        
        for log in logs {
            let schema_hash = log.target_schema;
            let payload = log.payload;
            
            // Find matching inbound table
            if let Some(table) = self.find_inbound_table(schema_hash) {
                // Decode payload according to table schema
                let data = table.abi_encoder.decode(&payload)?;
                
                // Insert into inbound table
                self.insert_inbound_message(&table.name, data).await?;
            }
        }
        
        self.last_processed_block = logs.last()
            .map(|l| l.block_number)
            .unwrap_or(self.last_processed_block);
            
        Ok(())
    }
}
```

### Message Validation

```rust
// validator/src/validation.rs
pub struct MessageValidator {
    rules: Vec<Box<dyn ValidationRule>>,
    limits: MessageLimits,
}

pub struct MessageLimits {
    max_withdrawal_amount: U256,
    daily_withdrawal_limit: U256,
    hourly_message_limit: usize,
    max_gas_per_call: u64,
}

#[async_trait]
pub trait ValidationRule: Send + Sync {
    async fn validate(&self, message: &Message, context: &ValidationContext) -> Result<()>;
}

pub struct WithdrawalLimitRule {
    daily_limit: U256,
    window: Duration,
}

#[async_trait]
impl ValidationRule for WithdrawalLimitRule {
    async fn validate(&self, message: &Message, context: &ValidationContext) -> Result<()> {
        if let Some(amount) = message.get_field("amount") {
            let amount: U256 = amount.parse()?;
            let account = message.get_field("account_id").unwrap();
            
            let daily_total = context.get_daily_total(account).await?;
            
            if daily_total + amount > self.daily_limit {
                return Err(ValidationError::DailyLimitExceeded);
            }
        }
        Ok(())
    }
}

pub struct SignatureVerificationRule;

#[async_trait]
impl ValidationRule for SignatureVerificationRule {
    async fn validate(&self, message: &Message, context: &ValidationContext) -> Result<()> {
        // Verify any application-level signatures
        if let Some(sig) = message.get_field("user_signature") {
            let signer = recover_signer(&message.hash(), &sig)?;
            let expected = message.get_field("account_id").unwrap();
            
            if !context.is_authorized(signer, expected).await? {
                return Err(ValidationError::UnauthorizedSigner);
            }
        }
        Ok(())
    }
}

pub struct AnomalyDetectionRule {
    ml_model: AnomalyDetector,
}

#[async_trait]
impl ValidationRule for AnomalyDetectionRule {
    async fn validate(&self, message: &Message, context: &ValidationContext) -> Result<()> {
        let features = self.extract_features(message, context).await?;
        let anomaly_score = self.ml_model.predict(&features)?;
        
        if anomaly_score > 0.95 {
            warn!("Anomalous message detected: {:?}", message);
            return Err(ValidationError::AnomalousPattern);
        }
        
        Ok(())
    }
}
```

### Multi-Validator Consensus

```rust
// validator/src/consensus.rs
pub struct ConsensusManager {
    validators: Vec<ValidatorEndpoint>,
    threshold: usize,
    timeout: Duration,
}

pub struct ValidatorEndpoint {
    url: String,
    public_key: PublicKey,
    tee_mrenclave: Option<[u8; 32]>,
}

impl ConsensusManager {
    pub async fn gather_signatures(
        &self,
        message: &ContractMessage,
        own_signature: Signature,
        own_attestation: Option<Attestation>
    ) -> Result<Vec<ValidatorSignature>> {
        let mut signatures = vec![ValidatorSignature {
            validator: self.own_address(),
            signature: own_signature.to_bytes(),
            attestation: own_attestation.map(|a| a.to_bytes()).unwrap_or_default(),
        }];
        
        // Request signatures from other validators
        let futures: Vec<_> = self.validators.iter().map(|validator| {
            self.request_signature(validator, message)
        }).collect();
        
        let results = futures::future::join_all(futures).await;
        
        for result in results {
            if let Ok(sig) = result {
                signatures.push(sig);
                
                if signatures.len() >= self.threshold {
                    break;  // We have enough signatures
                }
            }
        }
        
        if signatures.len() < self.threshold {
            return Err(ConsensusError::InsufficientSignatures);
        }
        
        Ok(signatures)
    }
    
    async fn request_signature(
        &self,
        validator: &ValidatorEndpoint,
        message: &ContractMessage
    ) -> Result<ValidatorSignature> {
        let client = reqwest::Client::new();
        
        let response = client
            .post(&format!("{}/sign", validator.url))
            .json(&SignatureRequest {
                message: message.clone(),
                requester: self.own_address(),
            })
            .timeout(self.timeout)
            .send()
            .await?;
            
        let sig_response: SignatureResponse = response.json().await?;
        
        // Verify the signature
        let signer = recover_signer(&message.hash(), &sig_response.signature)?;
        if signer != validator.public_key.to_address() {
            return Err(ConsensusError::InvalidSignature);
        }
        
        // Verify TEE attestation if provided
        if let Some(attestation) = &sig_response.attestation {
            self.verify_attestation(attestation, validator)?;
        }
        
        Ok(ValidatorSignature {
            validator: signer,
            signature: sig_response.signature,
            attestation: sig_response.attestation.unwrap_or_default(),
        })
    }
}
```

## Circuit Breakers and Safety

### Rate Limiting

```rust
// bridge/src/safety.rs
pub struct RateLimiter {
    limits: HashMap<String, Limit>,
    windows: HashMap<String, Window>,
}

pub struct Limit {
    max_count: usize,
    window_duration: Duration,
    max_amount: Option<U256>,
}

impl RateLimiter {
    pub fn check_withdrawal(&mut self, amount: U256, account: &str) -> Result<()> {
        // Check global daily limit
        self.check_limit("global_daily", amount)?;
        
        // Check per-account hourly limit
        self.check_limit(&format!("account_hourly_{}", account), amount)?;
        
        // Check velocity (sudden spike detection)
        self.check_velocity(amount, account)?;
        
        Ok(())
    }
    
    fn check_velocity(&self, amount: U256, account: &str) -> Result<()> {
        let history = self.get_history(account);
        let average = history.average();
        
        if amount > average * 10 {
            return Err(SafetyError::VelocityCheckFailed);
        }
        
        Ok(())
    }
}
```

### Pause Mechanisms

```rust
pub struct CircuitBreaker {
    paused: Arc<AtomicBool>,
    pause_reasons: Arc<Mutex<Vec<PauseReason>>>,
    auto_resume: Option<Duration>,
}

pub enum PauseReason {
    ManualPause,
    AnomalyDetected { details: String },
    ThresholdExceeded { metric: String, value: f64 },
    ExternalThreat { source: String },
}

impl CircuitBreaker {
    pub fn trip(&self, reason: PauseReason) {
        self.paused.store(true, Ordering::SeqCst);
        self.pause_reasons.lock().unwrap().push(reason);
        
        if let Some(duration) = self.auto_resume {
            tokio::spawn(async move {
                tokio::time::sleep(duration).await;
                self.reset();
            });
        }
    }
    
    pub fn check(&self) -> Result<()> {
        if self.paused.load(Ordering::SeqCst) {
            return Err(SafetyError::SystemPaused);
        }
        Ok(())
    }
}
```

## Testing Infrastructure

### Contract Tests

```javascript
// test/Bridge.test.js
const { expect } = require("chai");
const { ethers } = require("hardhat");

describe("Bridge", function () {
  let bridge;
  let validators;
  
  beforeEach(async function () {
    validators = await ethers.getSigners().slice(0, 3);
    const Bridge = await ethers.getContractFactory("Bridge");
    bridge = await Bridge.deploy();
    await bridge.initialize(
      validators.map(v => v.address),
      2,  // threshold
      ethers.utils.parseEther("1000")  // daily limit
    );
  });
  
  it("Should process withdrawal with valid signatures", async function () {
    const message = {
      id: 1,
      messageType: 0,  // WITHDRAWAL
      schemaHash: ethers.utils.keccak256(ethers.utils.toUtf8Bytes("outbound_withdrawals")),
      payload: ethers.utils.defaultAbiCoder.encode(
        ["address", "address", "uint256"],
        [user.address, ethers.constants.AddressZero, ethers.utils.parseEther("10")]
      ),
      nonce: 0,
      timestamp: Math.floor(Date.now() / 1000)
    };
    
    // Get signatures from validators
    const messageHash = ethers.utils.keccak256(ethers.utils.defaultAbiCoder.encode(
      ["uint256", "uint8", "bytes32", "bytes", "uint256", "uint256"],
      [message.id, message.messageType, message.schemaHash, message.payload, message.nonce, message.timestamp]
    ));
    
    const signatures = await Promise.all(
      validators.slice(0, 2).map(async (validator) => ({
        validator: validator.address,
        signature: await validator.signMessage(messageHash),
        attestation: "0x"
      }))
    );
    
    await bridge.processMessage(message, signatures);
    
    expect(await bridge.processedMessages(1)).to.be.true;
  });
  
  it("Should enforce daily withdrawal limit", async function () {
    // Process withdrawal up to limit
    // ...
    
    // Next withdrawal should fail
    await expect(
      bridge.processMessage(largeWithdrawal, signatures)
    ).to.be.revertedWith("Daily limit exceeded");
  });
  
  it("Should handle circuit breaker", async function () {
    await bridge.pause();
    
    await expect(
      bridge.processMessage(message, signatures)
    ).to.be.revertedWith("Pausable: paused");
    
    await bridge.unpause();
    
    // Should work after unpause
    await bridge.processMessage(message, signatures);
  });
});
```

### Integration Tests

```rust
#[tokio::test]
async fn test_end_to_end_withdrawal() {
    // Setup test environment
    let db = setup_test_db().await;
    let bridge = deploy_test_bridge().await;
    let processor = MessageProcessor::new(db, bridge);
    
    // Insert withdrawal message
    db.execute(
        "INSERT INTO outbound_withdrawals (account_id, recipient_address, token_address, amount) 
         VALUES ('alice', '0x...', '0x...', '1000000000000000000')",
        []
    ).unwrap();
    
    // Process messages
    processor.process_pending_messages().await.unwrap();
    
    // Verify on-chain
    let processed = bridge.processed_messages(1).await.unwrap();
    assert!(processed);
    
    // Verify database updated
    let status: String = db.query_row(
        "SELECT status FROM outbound_withdrawals WHERE id = 1",
        [],
        |row| row.get(0)
    ).unwrap();
    assert_eq!(status, "confirmed");
}
```

## Deployment Configuration

### Bridge Deployment Script

```javascript
// scripts/deploy-bridge.js
const hre = require("hardhat");

async function main() {
  const validators = [
    "0x...",  // Validator 1 (TEE)
    "0x...",  // Validator 2 (TEE)
    "0x...",  // Validator 3 (TEE)
  ];
  
  const threshold = 2;
  const dailyLimit = ethers.utils.parseEther("10000");
  
  // Deploy implementation
  const Bridge = await hre.ethers.getContractFactory("Bridge");
  const implementation = await Bridge.deploy();
  await implementation.deployed();
  
  // Deploy proxy
  const Proxy = await hre.ethers.getContractFactory("TransparentUpgradeableProxy");
  const proxy = await Proxy.deploy(
    implementation.address,
    proxyAdmin.address,
    Bridge.interface.encodeFunctionData("initialize", [validators, threshold, dailyLimit])
  );
  await proxy.deployed();
  
  console.log("Bridge deployed to:", proxy.address);
  
  // Verify on Etherscan
  await hre.run("verify:verify", {
    address: implementation.address,
    constructorArguments: [],
  });
}

main().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
```

### Validator Configuration

```yaml
# validator-config.yaml
bridge:
  contract_address: "0x..."
  chain_id: 1
  rpc_url: "https://eth-mainnet.g.alchemy.com/v2/..."
  
message_tables:
  - name: "outbound_withdrawals"
    type: "WITHDRAWAL"
    columns:
      - name: "recipient_address"
        sql_type: "TEXT"
        abi_type: "address"
      - name: "token_address"
        sql_type: "TEXT"  
        abi_type: "address"
      - name: "amount"
        sql_type: "TEXT"
        abi_type: "uint256"
        
  - name: "outbound_messages"
    type: "CALL"
    columns:
      - name: "target_address"
        sql_type: "TEXT"
        abi_type: "address"
      - name: "function_signature"
        sql_type: "TEXT"
        abi_type: "bytes4"
      - name: "parameters"
        sql_type: "BLOB"
        abi_type: "bytes"
        
validation_rules:
  - type: "withdrawal_limit"
    daily_limit: "1000000000000000000000"  # 1000 ETH
    
  - type: "signature_verification"
    required_signatures: 2
    
  - type: "anomaly_detection"
    model_path: "/models/anomaly_detector.pkl"
    threshold: 0.95
    
consensus:
  validators:
    - url: "https://validator1.synddb.io"
      public_key: "0x..."
      
    - url: "https://validator2.synddb.io"  
      public_key: "0x..."
      
  threshold: 2
  timeout_secs: 30
```

## Security Considerations

### 1. Message Authentication

All messages must be authenticated:

```rust
pub fn verify_message_origin(message: &Message, db: &Connection) -> Result<()> {
    // Verify message exists in database
    let exists: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM ? WHERE id = ?)",
        params![message.table_name, message.id],
        |row| row.get(0)
    )?;
    
    if !exists {
        return Err(SecurityError::MessageNotFound);
    }
    
    // Verify message hasn't been tampered with
    let hash = compute_message_hash(message);
    let stored_hash: Vec<u8> = db.query_row(
        "SELECT hash FROM ? WHERE id = ?",
        params![message.table_name, message.id],
        |row| row.get(0)
    )?;
    
    if hash != stored_hash {
        return Err(SecurityError::MessageTampered);
    }
    
    Ok(())
}
```

### 2. Replay Protection

Prevent message replay attacks:

```solidity
mapping(uint256 => bool) public processedMessages;
mapping(uint256 => uint256) public messageNonces;

function processMessage(Message calldata message, ...) external {
    require(!processedMessages[message.id], "Already processed");
    require(message.nonce == messageNonces[message.schemaHash]++, "Invalid nonce");
    processedMessages[message.id] = true;
}
```

### 3. Schema Validation

Validate message schemas:

```rust
pub fn validate_schema(message: &Message, expected: &TableSchema) -> Result<()> {
    let actual_schema = extract_schema(message)?;
    
    if actual_schema.hash() != expected.hash() {
        return Err(ValidationError::SchemaMismatch);
    }
    
    // Validate each field
    for field in expected.columns {
        if field.required && !message.has_field(&field.name) {
            return Err(ValidationError::MissingRequiredField(field.name));
        }
        
        if let Some(value) = message.get_field(&field.name) {
            validate_field_type(value, &field.abi_type)?;
        }
    }
    
    Ok(())
}
```

### 4. TEE Attestation Verification

Verify validator TEE attestations:

```rust
pub fn verify_validator_attestation(
    validator: &Address,
    attestation: &Attestation,
    expected_mrenclave: &[u8; 32]
) -> Result<()> {
    // Verify quote signature
    let quote = parse_quote(&attestation.quote)?;
    verify_quote_signature(&quote)?;
    
    // Check MRENCLAVE matches
    if quote.mrenclave != expected_mrenclave {
        return Err(SecurityError::InvalidMrenclave);
    }
    
    // Verify validator key is in quote
    let report_data = quote.report_data;
    let key_hash = keccak256(&validator.to_bytes());
    
    if report_data[..32] != key_hash {
        return Err(SecurityError::KeyNotInQuote);
    }
    
    // Check quote is recent
    if quote.timestamp < SystemTime::now() - Duration::from_secs(3600) {
        return Err(SecurityError::StaleQuote);
    }
    
    Ok(())
}
```

## Performance Optimizations

### 1. Batch Message Processing

```rust
pub async fn process_message_batch(messages: Vec<Message>) -> Result<Vec<TxHash>> {
    // Group messages by type for batch processing
    let mut grouped: HashMap<MessageType, Vec<Message>> = HashMap::new();
    for msg in messages {
        grouped.entry(msg.message_type).or_default().push(msg);
    }
    
    // Process each group in parallel
    let futures: Vec<_> = grouped.into_iter().map(|(msg_type, msgs)| {
        tokio::spawn(async move {
            process_typed_batch(msg_type, msgs).await
        })
    }).collect();
    
    let results = futures::future::join_all(futures).await;
    
    Ok(results.into_iter().flatten().collect())
}
```

### 2. Signature Aggregation

Use BLS signatures for aggregation:

```rust
pub struct AggregatedSignature {
    signature: BlsSignature,
    public_keys: Vec<BlsPublicKey>,
}

impl AggregatedSignature {
    pub fn aggregate(signatures: Vec<BlsSignature>) -> Self {
        let aggregated = bls::aggregate(&signatures);
        Self {
            signature: aggregated,
            public_keys: extract_public_keys(signatures),
        }
    }
    
    pub fn verify(&self, message: &[u8]) -> bool {
        bls::verify_aggregated(
            &self.signature,
            message,
            &self.public_keys
        )
    }
}
```

### 3. Optimistic Processing

Process messages optimistically:

```rust
pub async fn optimistic_process(message: Message) -> Result<()> {
    // Submit to bridge optimistically
    let tx = submit_to_bridge(message.clone()).await?;
    
    // Validate in parallel
    tokio::spawn(async move {
        if let Err(e) = validate_message(message).await {
            // Revert if validation fails
            revert_message(tx).await?;
        }
    });
    
    Ok(())
}
```

## Monitoring and Observability

### Metrics

```rust
lazy_static! {
    static ref MESSAGE_COUNTER: IntCounterVec = register_int_counter_vec!(
        "synddb_messages_total",
        "Total messages processed",
        &["type", "status"]
    ).unwrap();
    
    static ref MESSAGE_LATENCY: HistogramVec = register_histogram_vec!(
        "synddb_message_latency_seconds",
        "Message processing latency",
        &["type"]
    ).unwrap();
    
    static ref BRIDGE_BALANCE: GaugeVec = register_gauge_vec!(
        "synddb_bridge_balance",
        "Bridge contract balance",
        &["token"]
    ).unwrap();
}
```

### Alerts

```yaml
# prometheus-alerts.yaml
groups:
  - name: bridge_alerts
    rules:
      - alert: HighMessageLatency
        expr: synddb_message_latency_seconds{quantile="0.99"} > 60
        annotations:
          summary: "Message processing taking too long"
          
      - alert: LowBridgeBalance
        expr: synddb_bridge_balance{token="ETH"} < 10
        annotations:
          summary: "Bridge ETH balance below 10"
          
      - alert: ValidatorConsensusFailure
        expr: rate(synddb_consensus_failures[5m]) > 0.1
        annotations:
          summary: "Validator consensus failing"
```

## Migration Guide

### Adding Custom Message Types

1. **Define SQL Table**:
```sql
CREATE TABLE my_custom_messages (
    id INTEGER PRIMARY KEY,
    custom_field TEXT NOT NULL,
    status TEXT DEFAULT 'pending'
);
```

2. **Register with Validator**:
```yaml
message_tables:
  - name: "my_custom_messages"
    type: "CUSTOM"
    schema_hash: "0x..."
    columns:
      - name: "custom_field"
        abi_type: "string"
```

3. **Deploy Handler Contract**:
```solidity
contract MyCustomHandler is IMessageHandler {
    function handleMessage(bytes calldata payload) external override {
        string memory customField = abi.decode(payload, (string));
        // Handle custom logic
    }
}
```

4. **Register Handler**:
```javascript
await bridge.registerHandler(schemaHash, handler.address);
```

### Upgrading Bridge Contract

1. **Deploy New Implementation**:
```javascript
const BridgeV2 = await ethers.getContractFactory("BridgeV2");
const newImpl = await BridgeV2.deploy();
```

2. **Upgrade Proxy**:
```javascript
await proxyAdmin.upgrade(proxyAddress, newImpl.address);
```

3. **Run Migration**:
```javascript
const bridge = BridgeV2.attach(proxyAddress);
await bridge.migrate();
```

## Future Enhancements

### 1. Cross-Chain Message Routing
- Support for IBC, LayerZero, Axelar
- Automatic route optimization
- Multi-hop message passing

### 2. Advanced Oracle Integration
- Multiple oracle providers
- Consensus-based oracle responses
- Custom oracle networks

### 3. Programmable Message Handlers
- WASM-based custom handlers
- Hot-swappable message processors
- Dynamic schema evolution

### 4. Enhanced Security
- Multi-party computation for signatures
- Homomorphic encryption for private messages
- Zero-knowledge proofs for message validity
