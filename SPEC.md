# SyndDB: High-Performance Blockchain Database

## Overview

SyndDB enables developers to build high-performance blockchain applications using **any programming language** with SQLite bindings. Instead of learning a new framework, developers write applications in their preferred language (Python, JavaScript, Go, Rust, etc.) that persist data to SQLite, while SyndDB infrastructure automatically captures and publishes the SQL operations for verification and replication.

The key insight is that **SQL operations themselves become the verifiable audit trail**. The sequencer runs inside a TEE with attestations proving it's running the correct code, while validators (also in TEEs) verify the SQL statements and their effects. Rather than requiring validators to re-execute complex business logic and external API calls, they focus on auditing database operations, making verification practical without sacrificing application flexibility.

The architecture is simple:

1. **Write applications in any language** that use SQLite for persistence
2. **Run a sidecar listener** that monitors the SQLite database for changes
3. **Automatically publish SQL operations** to DA layers for durability and verification
4. **Enable permissionless read replicas** that sync the SQL operations to serve queries
5. **Support message passing** through application-defined tables that trigger cross-chain operations

Developers don't need to change how they build applications - just ensure all state changes are persisted to SQLite. This design delivers ultra-low latency (<1ms local writes) and high throughput while maintaining verifiability at the SQL level. Applications can use any programming language, frameworks, libraries, or external services - as long as the results are persisted to SQLite, the system captures everything needed for verification.

## Why SyndDB?

**Use any language, get blockchain verifiability for free.** SyndDB lets you build high-performance applications in Python, JavaScript, Go, Rust, or any language with SQLite - no framework to learn, no code changes required. Get offchain performance (sub-millisecond latency, unlimited throughput) with onchain transparency (all state verifiable through SQL operations). Your application runs in a TEE for accountability, while validators provide additional guardrails before settlement. Perfect for applications where ultra-low latency matters more than full decentralization: orderbooks, gaming, social feeds, real-time analytics.

### Why SQLite?

SQLite is the ideal foundation for verifiable blockchain applications:

- **Trivial TEE Colocation**: Runs in-process with your application - no separate database server, simple to deploy together inside a single TEE
- **Deterministic Execution**: Same SQL operations always produce the same results, making verification straightforward
- **High Performance**: Zero-copy reads, sub-millisecond writes, millions of operations per second
- **Extensible**: Add custom business logic via user-defined functions and triggers without changing application code
- **Universal Support**: Available in virtually every programming language (Python, JavaScript, Go, Rust, Java, C++, etc.)
- **Proven Stability**: Battle-tested database engine with billions of deployments worldwide

## Key Benefits

1. **Use any language and framework** - Write applications in Python, JavaScript, Go, Rust, or any language with SQLite bindings. Zero code changes required to adopt SyndDB if you already use SQLite.
2. **Incredibly fast and low latency** - Sub-millisecond local writes with high throughput
3. **Flexible asset management** - Assets can either:
   - Live natively on the system for maximum performance, or
   - Remain on the settlement chain with actions triggered via message passing
   - Bridge only when needed (bridging is optional, not required)
4. **Built-in indexing** - Few to no indexing requirements (indexing is built into the relational database)

## Trade-offs

For this performance, applications must accept:

1. **Centralized application instance** (same trust model as rollup sequencers, but with better performance)
   - Liveness depends on the application instance staying online
   - Fallback instances may restart from last published state (potential data loss between publications)
   - Validators remain decentralized for security
2. **Non-EVM execution framework** - Uses SQL instead of Solidity/EVM
3. **Asset location flexibility** comes with different trade-offs:
   - Assets on settlement layer: Maximum security but requires message passing for actions
   - Assets native on SyndDB: Maximum performance but relies on application and validator security model
   - Hybrid approach: Bridge assets as needed for specific operations (adds operational complexity)

## Architecture Overview

SyndDB makes any SQLite application blockchain-verifiable by automatically capturing changesets (logical database operations) and publishing them with periodic snapshots.

### Core Components

1. **Application - Any Language**
   - Written in any language (Python, Node.js, Go, Rust, Java, etc.)
   - Uses SQLite for persistence (via language-specific SQLite bindings)
   - Runs inside a TEE for attestation and accountability
   - Can use any libraries, frameworks, or external APIs
   - All state changes must be persisted to SQLite

2. **Sidecar Listener**
   - Attaches to the SQLite database via Session Extension
   - Captures deterministic changesets (logical INSERT/UPDATE/DELETE operations)
   - Creates periodic snapshots for recovery points
   - Batches changesets into diffs, publishes with snapshots to DA layers
   - Publishes to DA layers (Celestia, EigenDA) and storage layers (IPFS, Arweave)
   - Zero application code changes required

3. **Read Replicas**
   - Anyone can run a read replica permissionlessly
   - Sync changesets and snapshots from DA layers
   - Replay changesets to maintain consistent database state
   - Serve queries with full SQL capabilities

4. **Validators**
   - Read replicas with validation logic that runs in TEEs
   - Verify changesets before signing for settlement
   - Default checks: Changeset validity, state consistency, balance invariants
   - Optional checks: External API verification, custom business rules
   - Process cross-chain messages from message tables

5. **Bridge (Message Passing)**
   - Smart contract for processing cross-chain messages
   - Monitors message tables with schemas tied to contract ABI
   - Processes outbound messages (withdrawals, cross-chain calls)
   - Receives inbound messages (deposits, cross-chain responses)

### Data Flow

```
Application (Any Language) → SQLite → Sidecar → DA Layers ← Validators (TEE) → Blockchain → Bridge.sol
       in TEE                           ↓                        ↓
                                 Message Tables           Message Verification
```

**Application Path**: App writes to SQLite → Sidecar publishes to DA layers

**Validator Path**: Validators read from DA → Verify SQL → Post to blockchain

**Message Passing**: Validators detect messages in SQL → Process via Bridge.sol

Validators can subscribe to the sidecar (with TEE attestation) for lower latency instead of waiting for DA publication. The application never touches the blockchain directly.

### Application as Source of Truth

The application runs in a TEE with hardware attestations proving it's running the correct code. This provides accountability while maintaining performance.

Key properties:

- **TEE Accountability**: Attestations prove the application is running unmodified code
- **Validator Guardrails**: Validators enforce limits (withdrawal caps, rate limits) before settlement
- **Flexible Implementation**: Use any language, external APIs, or libraries - only SQL output matters
- **Performance Optimized**: Can prune historical data while providing snapshots for bootstrapping

This trades full decentralization for extreme performance, suitable for applications where ultra-low latency matters more than fully decentralized execution.

## Verifiability Model: SQL as the Audit Trail

Unlike traditional rollups that require full re-execution of all logic, SyndDB uses changesets (containing SQL operations) as the verifiable audit trail. This fundamental shift enables practical verifiability without sacrificing application flexibility.

### Four Pillars of SyndDB Verifiability

1. **Application Writes Everything to SQL**: The application must persist all data that could affect state transitions to SQLite, including:
   - Application state changes
   - Logs of external API calls and their results
   - User inputs and their effects
   - Message passing operations via special tables

2. **Sidecar Publishes to DA Layers**: The sidecar listener captures and publishes to censorship-resistant DA layers:
   - Changesets containing logical INSERT/UPDATE/DELETE operations with values
   - Sequence numbers maintaining strict ordering
   - Periodic snapshots for bootstrapping and recovery
   - Immediate snapshots on schema changes
   - This ensures data is widely available and reduces equivocation risk

3. **Validators Verify SQL, Not Code**: Validators read from DA layers and check:
   - SQL syntax and semantic correctness (default validation)
   - State transitions make sense (default validation)
   - Balances remain consistent (default validation)
   - Message passing operations follow rules (default validation)
   - Custom business logic checks (optional extensions)
   - Best-effort re-derivation of external data (optional extensions)

4. **Re-execution is Optional**: By default, validators don't need to:
   - Re-run the original application (which could be in any language)
   - Perfectly reproduce all external API calls
   - Re-compute complex algorithms exactly
   - Match non-deterministic operations

   However, validators can be extended to perform more thorough re-execution if desired - this is purely optional and application-specific.

### Why This Works

TEE attestations ensure the application runs the correct code. Validators verify the changesets (logical database operations), not the implementation. This is like database replication via changesets - apply the same principle to blockchain, with TEEs preventing equivocation and validators adding checks before settlement.

### Example: High-Performance Orderbook

**Traditional Onchain Approach** (validated but slow):

- Every order placement/cancellation is a blockchain transaction
- Matching logic runs in EVM
- Throughput limited to ~10-50 orders/second
- Transparent and verifiable, but impractical for high-frequency trading

**Traditional Offchain Approach** (fast but not validated):

- Orders processed in centralized database
- No transparency into matching logic
- High throughput, but trust the exchange operator

**SyndDB Approach** (best of both worlds):

```python
# High-performance matching engine in any language
def match_orders(buy_order, sell_order):
    # Complex matching logic with sub-millisecond latency
    # Use any libraries, optimizations, algorithms

    # Just persist the results to SQLite
    cursor.execute("""
        INSERT INTO trades (buy_order_id, sell_order_id, price, quantity)
        VALUES (?, ?, ?, ?)
    """, (buy_order.id, sell_order.id, match_price, quantity))

    cursor.execute("""
        UPDATE orders SET status = 'FILLED'
        WHERE id IN (?, ?)
    """, (buy_order.id, sell_order.id))

    db.commit()
```

**Benefits**:

- Offchain performance: Sub-millisecond order matching, unlimited throughput
- Onchain transparency: All trades verifiable through SQL operations
- Validator checks: Can add business logic like "no self-trading" or "price must be within spread" without re-implementing the matching engine
- Flexibility: Upgrade matching logic without changing validators

## Smart Contracts and Message Passing

### Publishing Model: Application → DA → Validators → Blockchain

The application never touches the blockchain - the sidecar publishes to DA layers:

0. **Application → SQLite**: Application writes all state changes to SQLite database
1. **Sidecar → DA**: Publishes SQL diffs/snapshots to DA/storage layers (Celestia, EigenDA, IPFS, Arweave) with TEE signatures
2. **Validators ← DA**: Validators sync from censorship-resistant DA layers
3. **Validators → Blockchain**: Post verified state transitions to settlement contract. Messages in the bridge tables are processed via Bridge.sol.

This keeps the application isolated from blockchain infrastructure while enabling multiple DA sources for resilience. No custom bridge code needed - just define tables that match your message schema.

### Validator Settlement Contract

Validators (not the application) interact with the blockchain:

```solidity
// Reference to DA layer data for transparency
// Changesets contain logical database changes (INSERT/UPDATE/DELETE operations)
function submitChangeset(string calldata daCid, bytes32 dataHash, uint256 sequenceNumber)

// Snapshots are full database states for recovery/bootstrapping
function submitSnapshot(string calldata daCid, bytes32 dataHash, uint256 sequenceNumber)
```

The settlement contract only accepts state roots signed by a threshold of validators running in TEEs.

### Message Passing Contract

Message passing operations are triggered when the application writes to message tables. The contract ABI is tied to the table schema, allowing flexible message types:

```solidity
// Generic message processing from outbound_messages table
function processMessage(
    uint256 messageId,
    bytes calldata messageData,  // Decoded from table schema
    bytes[] validatorSignatures
)

// Receives inbound messages and triggers sequencer to update inbound_messages table
function sendMessage(
    string calldata targetAccountId,
    bytes calldata messageData
)
```

Common message types include:

- Asset withdrawals/deposits (bridge operations)
- Cross-chain function calls
- Oracle data requests/responses
- Governance proposals

### Message Tables

Applications define message tables with schemas that map to contract ABIs:

```sql
-- Example: Asset withdrawal messages
CREATE TABLE outbound_withdrawals (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id TEXT NOT NULL,
    token_address TEXT NOT NULL,
    amount INTEGER NOT NULL,
    destination_address TEXT NOT NULL,
    status TEXT DEFAULT 'pending',
    created_at INTEGER DEFAULT (unixepoch())
);

-- Example: Inbound deposit messages
CREATE TABLE inbound_deposits (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    source_tx_hash TEXT UNIQUE NOT NULL,
    account_id TEXT NOT NULL,
    token_address TEXT NOT NULL,
    amount INTEGER NOT NULL,
    block_number INTEGER NOT NULL,
    created_at INTEGER DEFAULT (unixepoch())
);

-- Example: Generic cross-chain calls
CREATE TABLE outbound_messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    target_chain TEXT NOT NULL,
    target_contract TEXT NOT NULL,
    function_signature TEXT NOT NULL,
    parameters BLOB NOT NULL,
    status TEXT DEFAULT 'pending'
);
```

## Validators and Settlement

### Default Validator Implementation

SyndDB provides a default validator implementation that:

1. **Replays Changesets**: Applies changesets to rebuild state deterministically
2. **Verifies Bridge Claims**: Confirms all withdrawal/deposit amounts match bridge table entries
3. **Validates Basic Invariants**: Ensures balances never go negative, totals sum correctly
4. **Signs Valid States**: Approves states that pass all basic checks

### Extending the Default Validator

The default validator is designed to be extended with custom logic:

```rust
// Start with the default validator
use synddb_validator::DefaultValidator;

impl CustomValidator {
    fn validate(&self, changesets: &[Changeset]) -> Result<()> {
        // Run default validation first
        self.default.validate(changesets)?;

        // Add external API checks
        if self.config.check_oracles {
            self.verify_price_feeds(changesets)?;
        }

        // Add guardrails on amounts
        for changeset in changesets {
            if let Some(withdrawal) = parse_withdrawal(changeset) {
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
        self.check_kyc_requirements(changesets)?;
        self.verify_rate_limits(changesets)?;

        Ok(())
    }
}
```

### TEE Deployment (Required)

Validators **must** run in Trusted Execution Environments to ensure they are running unmodified code:

- **Attestation**: TEE validators prove they're running unmodified code
- **Protected Keys**: Settlement keys remain secure within the TEE
- **Changeset Verification**: Hardware-backed guarantees prevent validator subversion
- **Message Processing**: TEE validators can hold bridge signing authority

### Running the Default Validator

Deploy the default validator in a TEE:

```bash
# Run the default validator in TEE
synddb-validator \
    --da-layer celestia \
    --chain-rpc https://... \
    --mode default \
    --tee-attestation-key /path/to/key
```

The default validator will sync changesets and snapshots from DA layers, apply changesets to rebuild state, and verify basic invariants before signing for settlement.

Applications can be in any language, while validators share the same implementation to ensure consistent verification across the network.

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
- **Automatic Publishing**: The sidecar handles all DA layer and blockchain interaction
- **Consistent Validation**: Validators use the same implementation for verification
- **Permissionless Replication**: Anyone can sync and query the data
- **Optional Message Passing**: Add message tables only if you need cross-chain operations

### Migration from Existing Applications

Converting any existing SQLite application to SyndDB is straightforward:

1. Ensure all state changes go through SQLite (not just in-memory)
2. Add message tables if you need cross-chain message passing functionality
3. Deploy the sidecar listener alongside your application
4. No code changes required to your business logic

This approach makes SyndDB a drop-in solution for adding blockchain verifiability to applications written in any language.

## Terminology Glossary

### Core Architecture Terms

- **SyndDB** - Infrastructure that monitors applications (any language) using SQLite and publishes database operations to blockchain
- **Sidecar Listener** - Lightweight process that attaches to SQLite databases via Session Extension and automatically captures/publishes state changes as changesets
- **SQL Audit Trail** - The sequence of SQL operations that serves as the verifiable record of application state changes. SQLite executes deterministically, making all operations fully verifiable.
- **Changesets** - Deterministic logical database changes (INSERT/UPDATE/DELETE) captured via SQLite Session Extension, more compact and auditable than physical page changes
- **Snapshots** - Complete database state at a point in time, published periodically for recovery/bootstrapping and immediately on schema changes

### Node Types

- **Application** - Your application (any language) running inside a TEE with SQLite, publishing SQL operations via sidecar to DA layers
- **Read Replica** - Any node that syncs published SQL operations to serve queries (anyone can run permissionlessly)
- **Validator** - Read replica with additional validation logic that runs in a TEE and verifies SQL operations before signing for settlement

### State Management Terms

- **SQL Operations** - Database statements executed by the application and captured for verification
- **State Diff** - Batched changesets representing incremental logical database changes (not SQL statements, but INSERT/UPDATE/DELETE operations with values), published to DA layers
- **State Snapshot** - Complete SQLite database file at a specific version, published to DA layers for bootstrapping and recovery (also published immediately on schema changes)
- **Settlement** - Process where validators publish verified state to blockchain after reading from DA layers

### Message Passing Components

- **Message Tables** - Special SQLite tables for cross-chain operations (e.g., `outbound_messages`, `inbound_messages`) monitored by validators
- **Bridge.sol** - Smart contract that processes messages from message tables, with ABI tied to table schema
- **Message Passing** - Automatic detection and processing of cross-chain messages via application-defined table schemas that map to smart contract ABIs
