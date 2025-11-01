# SyndDB: High-Performance Blockchain Database

## Terminology Glossary

### Core Architecture Terms

- **SyndDB** - Infrastructure that monitors applications (any language) using SQLite and publishes database operations to blockchain
- **Sidecar Listener** - Lightweight process that attaches to SQLite databases and automatically captures/publishes state changes
- **SQL Audit Trail** - The sequence of SQL operations that serves as the verifiable record of application state changes

### Node Types

- **Sequencer** - Single trusted node running the application (any language) with SQLite, publishing SQL operations to blockchain
- **Read Replica** - Any node that syncs published SQL operations to serve queries (anyone can run permissionlessly)
- **Validator** - Standardized Rust-based read replica that verifies SQL operations and can process bridge operations (subset of read replicas)

### State Management Terms

- **SQL Operations** - Database statements executed by the application and captured for verification
- **State Diff** - SQL statements representing incremental database changes between versions
- **State Snapshot** - Complete SQLite database file at a specific version
- **Chain Submission** - Process of publishing SQL operations to blockchain for replication and verification

### Bridge Components

- **Bridge Tables** - Special SQLite tables (e.g., `bridge_withdrawals`, `bridge_deposits`) automatically monitored for cross-chain operations
- **Bridge.sol** - Smart contract that processes withdrawal/deposit requests from bridge tables
- **Message Passing** - Automatic detection and processing of bridge operations via standardized table schemas

## Overview

SyndDB enables developers to build high-performance blockchain applications using **any programming language** with SQLite. Instead of learning a new framework, developers write applications in their preferred language (Python, JavaScript, Go, Rust, etc.) that persist data to SQLite, while SyndDB infrastructure automatically captures and publishes the SQL operations for verification and replication.

The key insight is that **SQL operations themselves become the verifiable audit trail**. Rather than requiring validators to re-execute complex business logic and external API calls, they verify the SQL statements and their effects. This practical approach recognizes that the sequencer is already trusted for execution, so validators focus on auditing database operations rather than re-deriving them.

The architecture is simple:

1. **Write applications in any language** that use SQLite for persistence
2. **Run a sidecar listener** that monitors the SQLite database for changes
3. **Automatically publish SQL operations** to blockchain for durability and verification
4. **Enable permissionless read replicas** that sync the SQL operations to serve queries
5. **Support bridge operations** through standardized tables that trigger cross-chain messages

This design delivers ultra-low latency (<1ms local writes) and high throughput while maintaining verifiability at the SQL level. Applications can use any programming language, frameworks, libraries, or external services - as long as the results are persisted to SQLite, the system captures everything needed for verification.

## Key Benefits

1. Incredibly fast and low latency system
2. Flexible asset management - assets can either:
   - Live natively on the system for maximum performance, or
   - Remain on the settlement chain with actions triggered via message passing
   - Bridge assets only when needed (bridging is optional, not required)
3. Few to no indexing requirements (indexing is built into the relational database)

## Trade-offs

For this performance, applications must accept:

1. Significant decentralization trade-offs in block production (decentralization in validators is maintained)
   - Single sequencer architecture means liveness failures if the sequencer goes down
   - Fallback sequencers must restart from last published state, not the sequencer's current state (potential data loss between publications)
2. Non-EVM execution framework
3. Asset location flexibility comes with different trade-offs:
   - Assets on settlement layer: Maximum security but requires message passing for actions
   - Assets native on SyndDB: Maximum performance but relies on sequencer and validator security model
   - Hybrid approach: Bridge assets as needed for specific operations (adds operational complexity)

## Architecture Overview

SyndDB transforms any standard Rust + SQLite application into a blockchain-verifiable system through automatic SQL operation capture and publication.

### Core Components

1. **Application (Sequencer) - Any Language**
   - Application written in any language (Python, Node.js, Go, Rust, Java, etc.)
   - Uses SQLite for persistence (via language-specific SQLite bindings)
   - Runs as a single trusted sequencer node
   - Can use any libraries, frameworks, or external APIs
   - All state changes must be persisted to SQLite

2. **Sidecar Listener**
   - Lightweight process that attaches to the SQLite database
   - Monitors all SQL operations via WAL (Write-Ahead Logging) or triggers
   - Batches and compresses SQL statements
   - Publishes to blockchain or IPFS/Arweave automatically
   - No code changes required in the application

3. **Read Replicas**
   - Anyone can run a read replica permissionlessly
   - Sync SQL operations from blockchain
   - Replay operations to maintain consistent database state
   - Serve queries with full SQL capabilities

4. **Validators (Optional) - Standardized in Rust**
   - Subset of read replicas running standardized Rust validation logic
   - Verify SQL operations and their results deterministically
   - Can make best-effort attempts to re-derive external API data
   - Process bridge operations from special tables
   - Add custom business logic checks before signing
   - May operate in TEEs for additional security

### Data Flow

```
App (Any Language) → SQLite → Sidecar Listener → Blockchain → Read Replicas → Queries
                            ↓                              ↓
                      Bridge Tables                 Validators (Rust)
                            ↓                              ↓
                       Bridge.sol              Settlement Verification
```

This architecture treats SQL as the universal language for state verification, eliminating the need for custom frameworks or execution environments. The sequencer can be in any language, while validators use standardized Rust code for consistent verification.

### Sequencer as Source of Truth

The SQLite database managed by the sequencer serves as the trusted source of truth in SyndDB's model. The sequencer operates as:

- **Source of Truth**: The sequencer runs application code in any language and publishes all SQL operations for verification
- **Trusted Role with Guardrails**: While trust is placed in the sequencer, circuit breakers (e.g., caps on withdrawals, pool limits, or throttling of asset movements) enforce safety
- **Flexible Business Logic**: The sequencer can use any programming language, external APIs, complex computations - only the SQL results matter
- **Application-Specific Logic**: The sequencer can prune historical data for performance while still providing snapshots for bootstrapping

This model trades full decentralization for practical high-performance guarantees, making it suitable for applications where ultra-low latency and throughput matter more than trustless derivability of all history.

## Verifiability Model: SQL as the Audit Trail

Unlike traditional rollups that require full re-execution of all logic, SyndDB uses SQL operations as the verifiable audit trail. This fundamental shift enables practical verifiability without sacrificing application flexibility.

### How It Works

1. **Sequencer Writes Everything to SQL**: The sequencer must persist all decision-relevant data to SQLite, including:
   - Application state changes
   - External data that influenced decisions
   - User inputs and their effects
   - Bridge operations via special tables

2. **SQL Operations Get Published**: The sidecar listener captures and publishes:
   - Every INSERT, UPDATE, DELETE operation
   - Transaction boundaries (BEGIN/COMMIT)
   - The ordering of all operations
   - Optional: Periodic state snapshots

3. **Validators Verify SQL, Not Code**: Standardized Rust validators check:
   - SQL syntax and semantic correctness
   - State transitions make sense
   - Balances remain consistent
   - Bridge operations follow rules
   - Custom business logic checks as needed
   - Optional: Best-effort re-derivation of external data for additional verification

4. **No Full Re-execution Required**: Validators don't need to:
   - Re-run the original application (which could be in any language)
   - Perfectly reproduce all external API calls
   - Re-compute complex algorithms exactly
   - Match non-deterministic operations

### Why This Works

The sequencer is already trusted for:

- Ordering transactions
- Running the business logic
- Deciding on state transitions

So instead of trying to make everything deterministic and re-executable, we focus verification on what matters: **the SQL operations that change state**. This is similar to how traditional databases use write-ahead logs for replication - we're applying the same principle to blockchain verification.

### Example: Oracle Price Updates

Traditional approach (complex):

```rust
// Fetch price from multiple sources
// Apply complex aggregation logic
// Handle failures and retries
// All of this needs to be deterministic!
```

SyndDB approach (simple):

```python
# Python example - fetch prices however you want
price = fetch_and_aggregate_prices()

# Just write the result to SQL
cursor.execute(
    "INSERT INTO prices (asset, price, timestamp) VALUES (?, ?, ?)",
    (asset, price, timestamp)
)
db.commit()
```

Validators only verify the SQL operation, not how the price was derived. They can add business logic checks (e.g., "price shouldn't change by >10% in one update") without re-implementing the oracle logic.

## Smart Contracts and Bridge Operations

### State Publication Contract

The sidecar listener automatically publishes SQL operations through a simple contract interface:

```solidity
// Publish SQL operations directly onchain (for small batches)
function publishSQLBatch(bytes calldata sqlOperations, uint256 sequenceNumber)

// Publish pointer to SQL operations stored offchain (for large batches)
function publishSQLPointer(string calldata ipfsCid, bytes32 operationsHash, uint256 sequenceNumber)

// Publish complete database snapshot for bootstrapping
function publishSnapshot(string calldata ipfsCid, bytes32 snapshotHash, uint256 blockHeight)
```

The sidecar listener handles all publication automatically - no application code changes needed.

### Bridge.sol for Asset Management

Bridge operations are triggered automatically when the application writes to special tables:

```solidity
// Monitors bridge_withdrawals table and processes approved withdrawals
function processWithdrawal(uint256 withdrawalId, address token, uint256 amount, address destination)

// Receives deposits and notifies sequencer to update bridge_deposits table
function deposit(address token, uint256 amount, string calldata accountId)
```

### Standard Bridge Tables

Applications implement bridge support by creating these standard tables:

```sql
-- Withdrawal requests written by application
CREATE TABLE bridge_withdrawals (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id TEXT NOT NULL,
    token_address TEXT NOT NULL,
    amount INTEGER NOT NULL,
    destination_address TEXT NOT NULL,
    status TEXT DEFAULT 'pending',
    created_at INTEGER DEFAULT (unixepoch()),
    processed_at INTEGER,
    tx_hash TEXT
);

-- Deposits detected by monitoring Bridge.sol events
CREATE TABLE bridge_deposits (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    source_tx_hash TEXT UNIQUE NOT NULL,
    account_id TEXT NOT NULL,
    token_address TEXT NOT NULL,
    amount INTEGER NOT NULL,
    block_number INTEGER NOT NULL,
    created_at INTEGER DEFAULT (unixepoch())
);
```

The sidecar listener monitors these tables and automatically coordinates with Bridge.sol - no custom bridge code needed.

### Example Database Transactions

#### Order Book Operations

**Example 1: Insert a new limit order:**

```sql
BEGIN;
INSERT INTO orders (order_id, account_id, side, price, quantity, status)
VALUES ('ord_123', 'acct_42', 'BUY', 42000, 0.5, 'OPEN');
COMMIT;
```

**Example 2: Match a trade between two orders:**

```sql
BEGIN;
UPDATE orders SET status = 'FILLED' WHERE order_id = 'ord_123';
UPDATE orders SET status = 'FILLED' WHERE order_id = 'ord_456';
INSERT INTO trades (trade_id, buy_order_id, sell_order_id, price, quantity)
VALUES ('trade_789', 'ord_123', 'ord_456', 42010, 0.5);
COMMIT;
```

**Example 3: Cancel an open order:**

```sql
BEGIN;
UPDATE orders SET status = 'CANCELED' WHERE order_id = 'ord_321';
COMMIT;
```

#### ERC-20 Token Operations

**Example 4: Mint new tokens:**

```sql
BEGIN;
-- Increase total supply
UPDATE token_metadata
SET total_supply = total_supply + 1000000
WHERE token_address = '0xabc...';

-- Credit recipient balance
INSERT INTO balances (account_id, token_address, balance)
VALUES ('acct_123', '0xabc...', 1000000)
ON CONFLICT (account_id, token_address)
DO UPDATE SET balance = balance + 1000000;

-- Record mint event
INSERT INTO transfer_events (event_id, token_address, from_address, to_address, amount, timestamp)
VALUES ('evt_456', '0xabc...', '0x0', 'acct_123', 1000000, 1698765432);
COMMIT;
```

**Example 5: Transfer tokens between accounts:**

```sql
BEGIN;
-- Debit sender
UPDATE balances
SET balance = balance - 500
WHERE account_id = 'acct_123' AND token_address = '0xabc...';

-- Credit recipient
INSERT INTO balances (account_id, token_address, balance)
VALUES ('acct_456', '0xabc...', 500)
ON CONFLICT (account_id, token_address)
DO UPDATE SET balance = balance + 500;

-- Record transfer event
INSERT INTO transfer_events (event_id, token_address, from_address, to_address, amount, timestamp)
VALUES ('evt_789', '0xabc...', 'acct_123', 'acct_456', 500, 1698765433);
COMMIT;
```

**Example 6: Bridge tokens out to settlement layer:**

```sql
BEGIN;
-- Burn tokens from sender
UPDATE balances
SET balance = balance - 1000
WHERE account_id = 'acct_123' AND token_address = '0xabc...';

-- Create withdrawal request for validator processing
INSERT INTO withdrawal_requests (request_id, account_id, token_address, amount, destination_address, status, timestamp)
VALUES ('withdraw_101', 'acct_123', '0xabc...', 1000, '0x789...', 'PENDING', 1698765434);

-- Record burn event
INSERT INTO transfer_events (event_id, token_address, from_address, to_address, amount, timestamp)
VALUES ('evt_234', '0xabc...', 'acct_123', '0x0', 1000, 1698765434);
COMMIT;
```

**Example 7: Bridge tokens in from settlement layer:**

```sql
BEGIN;
-- Process deposit from settlement layer (triggered by validator)
INSERT INTO balances (account_id, token_address, balance)
VALUES ('acct_789', '0xabc...', 2000)
ON CONFLICT (account_id, token_address)
DO UPDATE SET balance = balance + 2000;

-- Record deposit
INSERT INTO deposit_records (deposit_id, account_id, token_address, amount, source_tx_hash, timestamp)
VALUES ('deposit_202', 'acct_789', '0xabc...', 2000, '0xdef...', 1698765435);

-- Record mint event
INSERT INTO transfer_events (event_id, token_address, from_address, to_address, amount, timestamp)
VALUES ('evt_567', '0xabc...', '0x0', 'acct_789', 2000, 1698765435);
COMMIT;
```

## Validators and Settlement

### Default Validator Implementation

SyndDB provides a default validator implementation that:

1. **Replays SQL State Transitions**: Executes all SQL operations to rebuild state
2. **Verifies Bridge Claims**: Confirms all withdrawal/deposit amounts match bridge table entries
3. **Validates Basic Invariants**: Ensures balances never go negative, totals sum correctly
4. **Signs Valid States**: Approves states that pass all basic checks

### Extending the Default Validator

The default validator is designed to be extended with custom logic:

```rust
// Start with the default validator
use synddb_validator::DefaultValidator;

impl CustomValidator {
    fn validate(&self, sql_ops: &[SqlOp]) -> Result<()> {
        // Run default validation first
        self.default.validate(sql_ops)?;

        // Add external API checks
        if self.config.check_oracles {
            self.verify_price_feeds(sql_ops)?;
        }

        // Add guardrails on amounts
        for op in sql_ops {
            if let Some(withdrawal) = parse_withdrawal(op) {
                // Reject anomalous movements
                if withdrawal.amount > self.config.max_withdrawal {
                    return Err("Withdrawal exceeds maximum");
                }
                if withdrawal.amount > self.get_historical_average() * 10 {
                    return Err("Withdrawal 10x above average");
                }
            }
        }

        // Add any other custom logic
        self.check_kyc_requirements(sql_ops)?;
        self.verify_rate_limits(sql_ops)?;

        Ok(())
    }
}
```

### SQL Verification Without Re-execution

Validators verify SQL operations rather than re-executing application logic:

1. **SQL Audit**: Validators replay SQL operations to verify state transitions
2. **Extensible Checks**: Start with default validation, add custom rules as needed
3. **Optional External Verification**: Can attempt to verify external data sources
4. **Settlement Authority**: Approve bridge operations based on validation results

### TEE Deployment (Optional)

For additional security, validators can run in Trusted Execution Environments:

- **Attestation**: TEE validators prove they're running unmodified code
- **Protected Keys**: Settlement keys remain secure within the TEE
- **SQL Verification**: Same verification process, but with hardware-backed guarantees
- **Bridge Processing**: TEE validators can hold bridge signing authority

### Validator Types and Deployment

#### Running the Default Validator

The simplest deployment just runs the default validator:

```bash
# Run the default validator out-of-the-box
synddb-validator \
    --chain-rpc https://... \
    --mode default \
    --bridge-contract 0x123...
```

The default validator will:

- Sync and replay all SQL operations
- Verify bridge withdrawal/deposit amounts match
- Check basic invariants (no negative balances, etc.)
- Sign valid states for settlement

#### Custom Validator Extensions

Operators can extend the default validator:

```rust
// Custom validator with additional checks
use synddb_validator::{DefaultValidator, ValidatorConfig};

fn main() {
    let config = ValidatorConfig {
        // Enable optional features
        check_external_apis: true,
        max_withdrawal: 1_000_000,
        anomaly_detection: true,
        rate_limiting: true,
    };

    let validator = DefaultValidator::new(config)
        .with_price_oracle("https://api.oracle.com")
        .with_anomaly_threshold(10.0)  // 10x historical average
        .with_custom_check(my_custom_validation);

    validator.run().await?;
}

// Custom validation logic
fn my_custom_validation(sql_ops: &[SqlOp]) -> Result<()> {
    // Add any application-specific checks
    for op in sql_ops {
        // Custom business logic validation
    }
    Ok(())
}
```

While the sequencer can be in any language, all validators use the same Rust-based foundation to ensure consistent verification across the network.

## Use Cases

SyndDB is designed for high-scale applications that require ultra-low latency and high throughput, including:

- Onchain order books for perp DEXs
- Gaming state and leaderboards
- Social applications and feeds
- NFT marketplaces and metadata
- Real-time analytics and dashboards

## Implementation Guide

### Building a SyndDB Application

1. **Write Your Application in Any Language**

   **Python Example:**

   ```python
   import sqlite3
   from flask import Flask

   app = Flask(__name__)
   db = sqlite3.connect('app.db')

   @app.route('/trade')
   def execute_trade():
       # Your business logic here
       cursor = db.cursor()
       cursor.execute("INSERT INTO trades ...")
       db.commit()
   ```

   **Node.js Example:**

   ```javascript
   const Database = require("better-sqlite3");
   const express = require("express");

   const app = express();
   const db = new Database("app.db");

   app.post("/trade", (req, res) => {
     // Your business logic here
     db.prepare("INSERT INTO trades ...").run();
   });
   ```

   **Go Example:**

   ```go
   package main
   import (
       "database/sql"
       _ "github.com/mattn/go-sqlite3"
   )

   func main() {
       db, _ := sql.Open("sqlite3", "./app.db")
       // Your business logic here
       db.Exec("INSERT INTO trades ...")
   }
   ```

2. **Create Bridge Tables (if needed)**

   ```sql
   -- Add these tables if you need bridge functionality
   CREATE TABLE bridge_withdrawals (...);
   CREATE TABLE bridge_deposits (...);
   ```

3. **Deploy with Sidecar Listener**

   ```yaml
   # docker-compose.yml
   services:
     app:
       image: your-app:latest
       volumes:
         - ./data:/data

     synddb-sidecar:
       image: syndicate/synddb-listener:latest
       volumes:
         - ./data:/data
       environment:
         - DATABASE_PATH=/data/app.db
         - CHAIN_RPC=https://...
         - IPFS_GATEWAY=https://...
   ```

4. **Run Read Replicas**

   ```bash
   # Anyone can run a read replica
   synddb-replica \
     --chain-rpc https://... \
     --start-block 12345 \
     --serve-port 8080
   ```

5. **Optional: Deploy in TEE**
   ```bash
   # Run the entire stack in a TEE for additional security
   docker run --device /dev/sgx_enclave \
     -v /var/run/aesmd:/var/run/aesmd \
     your-app-tee:latest
   ```

### Key Design Principles

- **Language Agnostic**: Use any programming language, framework, or runtime
- **SQL as Truth**: All state changes must go through SQLite for capture
- **Automatic Publishing**: The sidecar handles all blockchain interaction
- **Standardized Validation**: Validators use consistent Rust code for verification
- **Permissionless Replication**: Anyone can sync and query the data
- **Optional Bridges**: Add bridge tables only if you need cross-chain operations

### Migration from Existing Applications

Converting any existing SQLite application to SyndDB is straightforward:

1. Ensure all state changes go through SQLite (not just in-memory)
2. Add bridge tables if you need withdrawal/deposit functionality
3. Deploy the sidecar listener alongside your application
4. No code changes required to your business logic

This approach makes SyndDB a drop-in solution for adding blockchain verifiability to applications written in any language.

## Summary: Why This Architecture Works

The shift to "language-agnostic applications with SQL verifiability" fundamentally simplifies SyndDB while making it more powerful:

### For Developers

- **Use Any Language**: Write applications in Python, JavaScript, Go, Rust, Java, or any language with SQLite support
- **Zero Learning Curve**: No framework to learn - just write to SQLite
- **Full Flexibility**: Use any libraries, frameworks, or external services
- **Easy Migration**: Existing applications just need a sidecar, no code rewrites
- **Local Development**: Test everything locally before deploying with blockchain

### For Validators

- **Default Implementation Provided**: Run a validator out-of-the-box with zero configuration
- **Extensible Architecture**: Start with default validation, add custom checks as needed
- **Standardized Verification**: All validators share the same Rust foundation for consistency
- **Simple SQL Checks**: Verify SQL operations, not polyglot application logic
- **Optional External Verification**: Can add checks for external APIs, anomalies, or custom business rules
- **No Language Dependencies**: Don't need to understand or run the original application's language
- **Clear Audit Trail**: SQL provides a universal, well-understood verification language

### For the Ecosystem

- **Practical Verifiability**: Focus verification on what matters (state changes)
- **High Performance**: Sub-millisecond latency with standard SQLite
- **Permissionless Access**: Anyone can run read replicas and query data
- **Bridge Compatibility**: Automatic cross-chain operations via standard tables

This architecture recognizes two key insights:

1. **The sequencer is already trusted for execution**, so we should optimize for developer experience and performance while maintaining verifiability at the SQL level.

2. **Applications can be in any language** while validators are standardized in Rust, creating a clean separation between business logic (any language) and verification logic (Rust).

By treating SQL as the universal audit trail rather than trying to make polyglot applications deterministically re-executable, SyndDB becomes both more practical and more powerful. Developers can use their favorite languages and frameworks, while validators provide consistent verification regardless of the implementation language.
