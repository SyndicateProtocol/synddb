# SyndDB: High-Performance Blockchain Database

## Overview
SyndDB is Syndicate's proposed high-performance blockchain database product. The goal is to extend the utility of blockchain infrastructure beyond traditional sequencing into database-scale workloads.

The core concept is to leverage blockchain infrastructure for SQLite-backed transaction indexing and high-throughput ingestion pipelines. By encoding database statements (e.g., SQL operations), compressing them, and storing them either onchain or referenced through IPFS/Arweave, SyndDB provides a trust-minimized yet practical model for scalable data persistence and replication.

By using relational databases, we get indexing "for free" (since relational databases are excellent for querying) and excellent performance with low latency. SQLite triggers and other standard programmability on top of the SQL database can be used as well. We turn the sequencer into an entity that can write to the database and validators into entities that can read from the database. This looks similar to existing rollups, but with SQL instead of the EVM as the execution framework.

This design balances performance and decentralization, recognizing that some use cases do not need decentralized block production and simply need decentralized validators. As a bonus, these use cases can tolerate historical data pruning while benefiting from ultra-low latency operations and the ability to preserve state.

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
2. Non-EVM execution framework
3. Asset location flexibility comes with different trade-offs:
   - Assets on settlement layer: Maximum security but requires message passing for actions
   - Assets native on SyndDB: Maximum performance but relies on sequencer and validator security model
   - Hybrid approach: Bridge assets as needed for specific operations (adds operational complexity)

## Architecture Overview
At a high level, SyndDB consists of two primary roles:
* **Sequencer** – A single trusted node that runs a local SQLite instance, sequences database transactions, batches state transitions, and publishes state diffs/state snapshots onchain or via offchain pointers (IPFS/Arweave).
* **Read Replica** – Any node that ingests the published state (state snapshots or state diffs) and reconstructs the database locally for query and indexing. **Anyone can run a read replica** to access the data and serve queries.

A subset of read replicas can optionally become validators:
* **Validator** – A specialized read replica that runs inside a TEE (Trusted Execution Environment) with additional capabilities for processing withdrawals and settlement to the blockchain. Not all read replicas need to be validators - most read replicas will simply serve queries and provide data access.

The ingestion pipeline resembles standard blockchain pipelines, but via a different execution framework. SQLite state snapshots can be used to bootstrap new participants, while state diffs allow continuous low-latency updates.

In SyndDB, sequencing works analogously to an appchain, but with database-native semantics:
* The single sequencer batches SQL statements and publishes them periodically to the blockchain.
* Read replicas (which anyone can run) sync to these updates, ensuring they have consistent state.
* Special database transaction types allow bridge operations (e.g., withdrawals, liquidations) from SyndDB into the broader blockchain ecosystem, processed only by the subset of read replicas that are validators.

This approach enables appchain-style composability while keeping the sequencer optimized for database workloads.

### Sequencer as Source of Truth

The SQLite database managed by the sequencer serves as the trusted source of truth in SyndDB's model. The sequencer operates as:
* **Source of Truth**: The sequencer orders database transactions in real-time and publishes batched updates.
* **Trusted Role with Guardrails**: While trust is placed in the sequencer, circuit breakers (e.g., caps on withdrawals, pool limits, or throttling of asset movements) enforce safety.
* **Application-Specific Logic**: The sequencer can prune historical data for performance (useful for perp DEX order books or ephemeral social feeds) while still providing state snapshots for bootstrapping.

This model trades full decentralization for practical high-performance guarantees, making it suitable for applications where ultra-low latency and throughput matter more than trustless derivability of all history.

### Smart Contracts for Ordering + State Derivation
Smart contracts define the interface for state publication with four primary functions:

```solidity
writeDiff(bytes32 diffHash, uint256 diffIndex, bytes calldata diff)
// Publish a state diff of SQL statements to blockchain. Can be chunked via an index as necessary

writeDiffPointer(bytes calldata cid)
// Publish a state diff to IPFS/Arweave, write the CID to blockchain for ordering

writeSnapshot(bytes32 snapshotHash, uint256 snapshotIndex, bytes calldata snapshot)
// Publish a state snapshot to blockchain. Can be chunked via an index as necessary

writeSnapshotPointer(bytes calldata cid)
// Publish a state snapshot to IPFS/Arweave, write the CID to blockchain for ordering
```

New read replicas can derive state either from genesis, or by bootstrapping from the latest state snapshot. The latter is likely a good fit for data that is frequently pruned, but deriving state all the way back from genesis can be used for historical data.

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

### TEE Validators for Settlement (Optional)
For applications requiring stronger guarantees (e.g., bridging assets, financial contracts), SyndDB can incorporate Trusted Execution Environments (TEEs) to create specialized validators. **Important: Only a subset of read replicas need to become validators.**

* **Validators are a Subset of Read Replicas**: While anyone can run a standard read replica for querying data, only specially designated read replicas running inside TEEs become validators with settlement authority.
* **Most Read Replicas are Not Validators**: The majority of read replica nodes will simply sync state and serve queries. They don't need TEE hardware or settlement responsibilities.
* **State Verification**: Validators use their TEE-protected database replica to verify derived state, checking validity from state snapshots or genesis.
* **Settlement Authority**: Only TEE validators hold settlement keys authorized via attestation verification on the settlement chain.
* **Bridge Operations**: Upon detecting special database transaction types (e.g., asset withdrawal requests), TEE validators (not regular read replicas) sign and submit settlement transactions through a bridge.

This design allows anyone to run a read replica for data access while limiting settlement authority to a trusted subset of TEE validators, balancing accessibility with security. Circuit breakers and safeguards (implemented in settlement-layer contracts) mitigate risk from the trusted sequencer role.

## Use Cases
SyndDB is designed for high-scale applications that require ultra-low latency and high throughput, including:
* Onchain order books for perp DEXs
* Gaming state and leaderboards
* Social applications and feeds
* NFT marketplaces and metadata
* Real-time analytics and dashboards

## Implementation Framework
### Core Components
The SyndDB framework provides:
* SQLite database bootstrapping via the sequencer
* SQLite message passing to the settlement chain via validators
* Smart contracts for state publication
* TEE-based validator infrastructure

### Application-Specific Implementation
Applications building on SyndDB are responsible for:
* Database schema design
* Specific SQL transactions
* Message passing format to/from the bridge
* Application-specific business logic
