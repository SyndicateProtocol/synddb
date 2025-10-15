# SyndDB: High-Performance Blockchain Database

## Overview
SyndDB is Syndicate's proposed high-performance blockchain database product. The goal is to extend the utility of blockchain infrastructure beyond traditional sequencing into database-scale workloads.

The core concept is to leverage blockchain infrastructure for SQLite-backed transaction indexing and high-throughput ingestion pipelines. By encoding database statements (e.g., SQL operations), compressing them, and storing them either onchain or referenced through IPFS/Arweave, SyndDB provides a trust-minimized yet practical model for scalable data persistence and replication.

By using relational databases, we get indexing "for free" (since relational databases are excellent for querying) and excellent performance with low latency. SQLite triggers and other standard programmability on top of the SQL database can be used as well. We turn the sequencer into an entity that can write to the database and validators into entities that can read from the database. This looks similar to existing rollups, but with SQL instead of the EVM as the execution framework.

This design balances performance and decentralization, recognizing that some use cases do not need decentralized block production and simply need decentralized validators. As a bonus, these use cases can tolerate historical data pruning while benefiting from ultra-low latency operations and the ability to preserve state.

## Key Benefits
1. Incredibly fast and low latency system
2. Simple settlement to/from blockchain pools (no need to bridge assets)
3. Few to no indexing requirements (indexing is built into the relational database)

## Trade-offs
For this performance, applications must accept:
1. Significant decentralization trade-offs in block production (decentralization in validators is maintained)
2. Non-EVM execution framework
3. Funds live on the settlement layer rather than the database layer

## Architecture Overview 
At a high level, SyndDB consists of two roles:
* **Writer (Leader)** – Runs a local SQLite instance, batches state transitions, and posts diffs/snapshots onchain or via offchain pointers (IPFS/Arweave).
* **Reader (Validator)** – Ingests the published state (snapshots or diffs) and reconstructs the database locally for query and indexing.

The ingestion pipeline resembles standard blockchain pipelines, but via a different execution framework. SQLite snapshots can be used to bootstrap new participants, while diffs allow continuous low-latency updates.

In SyndDB, sequencing works analogously to an appchain, but with database-native semantics:
* Writers batch SQL state transitions and post them periodically to the blockchain.
* Readers synchronize to these updates, ensuring they have consistent replicated state.
* Special transaction types allow message passing (e.g., withdrawals, liquidations) from SyndDB into the broader blockchain ecosystem.

This approach enables appchain-style composability while keeping the leader optimized for database workloads.

### Database as Leader

The SQLite database serves as the trusted sequencer and leader in SyndDB's model. The leader operates as:
* **Source of Truth**: Writers sequence transactions in real-time and post batched updates.
* **Trusted Role with Guardrails**: While trust is placed in the leader, circuit breakers (e.g., caps on withdrawals, pool limits, or throttling of asset movements) enforce safety.
* **Application-Specific Logic**: Leaders can prune historical data for performance (useful for perp DEX order books or ephemeral social feeds) while still providing snapshots for bootstrapping.

This model trades full decentralization for practical high-performance guarantees, making it suitable for applications where ultra-low latency and throughput matter more than trustless derivability of all history.

### Smart Contracts for Ordering + State Derivation
Smart contracts define the interface for state publication with four primary functions:

```solidity
writeDiff(bytes32 diffHash, uint256 diffIndex, bytes calldata diff) 
// Write a diff of SQLite transactions to blockchain. Can be chunked via an index as necessary

writeDiffPointer(bytes calldata cid) 
// Write a diff to IPFS/Arweave, write the CID to blockchain for ordering

writeSnapshot(bytes32 snapshotHash, uint256 snapshotIndex, bytes calldata snapshot) 
// Write a snapshot to blockchain. Can be chunked via an index as necessary

writeSnapshotPointer(bytes calldata cid) 
// Write a snapshot to IPFS/Arweave, write the CID to blockchain for ordering
```

New validators or read-only nodes can derive state either from genesis, or by getting the latest snapshot. The latter is likely a good fit for data that is frequently pruned, but indexing all the way back to genesis can be used for historical data.

### Example Transactions

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

### TEE Settlement via Validators
For applications requiring stronger guarantees (e.g., bridging assets, financial contracts), SyndDB can incorporate Trusted Execution Environments (TEEs) as validators. This design includes:
* A read replica inside a TEE verifies ingested state, checking validity from snapshots or Genesis.
* TEEs hold settlement keys authorized via zkVM on the settlement chain.
* Upon detecting special transaction types (e.g., asset withdrawal requests), the TEE replica signs and submits settlement messages through a bridge on the settlement chain.

This design secures settlement without requiring every read node to act as a validator, balancing scalability with security assurances. Circuit breakers and safeguards (implemented in settlement-layer contracts) mitigate risk from the trusted leader role.

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
* SQLite database bootstrapping via the leader
* SQLite message passing to the settlement chain via validators
* Smart contracts for reads/writes
* TEE-based validator infrastructure

### Application-Specific Implementation
Applications building on SyndDB are responsible for:
* Database schema design
* Specific SQL transactions
* Message passing format to/from the bridge
* Application-specific business logic
