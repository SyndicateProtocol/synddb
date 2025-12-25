# Prediction Market Example

A minimal prediction market application demonstrating SyndDB integration. This example is designed to evaluate the developer experience (DX) of integrating SyndDB into an application.

## Overview

This is a binary prediction market where:
- Markets have YES/NO outcomes
- Shares cost a fixed 50 cents each (simplified pricing)
- Winners receive $1.00 per share when the market resolves
- Deposits and withdrawals integrate with the chain monitor pattern

## Usage

### Without SyndDB (standalone mode)

```bash
# Initialize database
cargo run -p prediction-market -- init

# Create accounts
cargo run -p prediction-market -- create-account alice
cargo run -p prediction-market -- create-account bob

# Create a market
cargo run -p prediction-market -- create-market "Will ETH hit 5k in 2025?" --resolution-time 1767225600

# Buy shares
cargo run -p prediction-market -- buy --account 1 --market 1 --outcome yes --shares 100
cargo run -p prediction-market -- buy --account 2 --market 1 --outcome no --shares 50

# Check status
cargo run -p prediction-market -- status

# Resolve market
cargo run -p prediction-market -- resolve --market 1 --outcome yes

# Check final balances
cargo run -p prediction-market -- status
```

### With SyndDB replication

```bash
# Start sequencer first (in another terminal)
cargo run -p synddb-sequencer -- --signing-key 0000000000000000000000000000000000000000000000000000000000000001

# Run with sequencer URL
cargo run -p prediction-market -- --sequencer http://localhost:8433 create-account alice
cargo run -p prediction-market -- --sequencer http://localhost:8433 buy --account 1 --market 1 --outcome yes --shares 100
```

Or set the environment variable:
```bash
export SEQUENCER_URL=http://localhost:8433
cargo run -p prediction-market -- create-account alice
```

### Bridge operations (deposits/withdrawals)

```bash
# Simulate a deposit from L1 (in production, chain monitor does this)
cargo run -p prediction-market -- simulate-deposit \
    --tx-hash 0xabc123 \
    --from 0x1111111111111111111111111111111111111111 \
    --to 0x2222222222222222222222222222222222222222 \
    --amount 100000

# Process pending deposits (credits accounts)
cargo run -p prediction-market -- process-deposits

# Request a withdrawal to L1
cargo run -p prediction-market -- withdraw --account 1 --amount 50000 --destination 0x1234567890abcdef
```

## Chain Monitor Integration

This example includes optional chain monitor integration for watching L1 bridge events.

### Building with chain monitor

```bash
cargo build -p prediction-market --features chain-monitor
```

### Architecture

```
L1 Bridge Contract
       │
       ▼
┌─────────────────┐
│  Chain Monitor  │  (watches for Deposit/Withdrawal events)
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│    Handlers     │  (DepositHandler, WithdrawalHandler)
└────────┬────────┘
         │ channel
         ▼
┌─────────────────┐
│   Main Thread   │  (receives from channel, inserts into DB)
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│  SQLite Tables  │  (inbound_deposits, outbound_withdrawals)
└─────────────────┘
```

### Event Flow

**Deposits (L1 → L2):**
1. User deposits USDC to bridge contract on L1
2. Bridge emits `Deposit(from, to, amount, data)` event
3. Chain monitor's `DepositHandler` captures event
4. Handler sends deposit data via channel to main thread
5. Main thread inserts into `inbound_deposits` table
6. Application calls `process_deposits()` to credit accounts

**Withdrawals (L2 → L1):**
1. User requests withdrawal via `request_withdrawal()`
2. Record created in `outbound_withdrawals` with status 'pending'
3. Sequencer batches and submits to L1 (status → 'submitted')
4. Bridge processes withdrawal and emits `Withdrawal` event
5. Chain monitor's `WithdrawalHandler` captures event
6. Handler sends confirmation via channel to main thread
7. Main thread updates status to 'confirmed'

## Developer Experience Notes

### SyndDB Integration Patterns

The codebase demonstrates several SyndDB integration patterns:

1. **`SyndDB::open()` hides complexity**: No need for `Box::leak` pattern:
   ```rust
   // Clean one-liner
   let synddb = SyndDB::open("market.db", "http://sequencer:8433")?;
   let conn = synddb.connection();
   ```

2. **Transaction helper**: Wraps `unchecked_transaction` automatically:
   ```rust
   synddb.transaction(|tx| {
       tx.execute("INSERT INTO ...", [])?;
       Ok(())
   })?;
   ```

3. **Health/stats API**: Monitor replication status:
   ```rust
   if synddb.is_healthy() { ... }
   let stats = synddb.stats(); // pending, published, failed counts
   ```

4. **Publish strategies**: Control when changesets are sent:
   - `PublishStrategy::Timer` (default): Auto-publish every 1 second
   - `PublishStrategy::Manual`: Only on explicit `publish()` call

### Remaining Friction Points

1. **Schema evolution**: No built-in migration support. Applications must manage schema changes carefully since they're replicated.

2. **Chain monitor setup**: Requires WebSocket RPC URL and bridge contract address. Configuration can be complex for multi-chain setups.

## Schema

```sql
-- Markets (binary YES/NO outcomes)
CREATE TABLE markets (
    id INTEGER PRIMARY KEY,
    question TEXT NOT NULL,
    outcome TEXT DEFAULT 'unresolved',  -- 'yes', 'no', or 'unresolved'
    resolution_time INTEGER,
    ...
);

-- User accounts with balances
CREATE TABLE accounts (
    id INTEGER PRIMARY KEY,
    name TEXT UNIQUE NOT NULL,
    balance INTEGER DEFAULT 1000000,  -- in cents
    ...
);

-- Share positions per market/outcome
CREATE TABLE positions (
    account_id INTEGER,
    market_id INTEGER,
    outcome TEXT,  -- 'yes' or 'no'
    shares INTEGER,
    ...
);

-- Trade history
CREATE TABLE trades (...);

-- Chain monitor: Inbound deposits from L1
CREATE TABLE inbound_deposits (
    id INTEGER PRIMARY KEY,
    tx_hash TEXT UNIQUE NOT NULL,
    from_address TEXT NOT NULL,
    to_address TEXT NOT NULL,
    amount INTEGER NOT NULL,
    block_number INTEGER NOT NULL,
    log_index INTEGER,
    processed INTEGER DEFAULT 0,
    ...
);

-- Chain monitor: Outbound withdrawals to L1
CREATE TABLE outbound_withdrawals (
    id INTEGER PRIMARY KEY,
    account_id INTEGER NOT NULL,
    amount INTEGER NOT NULL,
    destination_address TEXT NOT NULL,
    status TEXT DEFAULT 'pending',  -- 'pending', 'submitted', 'confirmed'
    l1_tx_hash TEXT,
    confirmed_at INTEGER,
    ...
);
```

## Development Environment

The `scripts/dev-env.sh` script provides a complete local development environment that demonstrates the full SyndDB architecture:

```bash
# Run the full development environment
./examples/prediction-market/scripts/dev-env.sh

# Skip the chain monitor (if you don't have anvil/forge)
./examples/prediction-market/scripts/dev-env.sh --no-monitor

# Clean up data files
./examples/prediction-market/scripts/dev-env.sh --cleanup
```

### What It Does

1. **Starts Anvil** - Local Ethereum node for testing
2. **Deploys TestBridge** - Simple bridge contract that emits Deposit/Withdrawal events
3. **Starts Sequencer** - With local SQLite storage (`publisher_type=local`)
4. **Runs Prediction Market** - Creates accounts, markets, trades
5. **Starts Validator** - Fetches from sequencer, reconstructs state, verifies signatures
6. **Emits Bridge Events** - For chain monitor testing

### Requirements

- Rust toolchain (cargo)
- Foundry (anvil, forge, cast) - Install with `curl -L https://foundry.paradigm.xyz | bash && foundryup`

### Architecture Diagram

```
┌─────────────┐      ┌─────────────┐      ┌─────────────┐
│   Anvil     │◄────►│ TestBridge  │      │   Chain     │
│  (L1 node)  │      │  Contract   │─────►│  Monitor    │
└─────────────┘      └─────────────┘      └──────┬──────┘
                                                  │
                     ┌─────────────┐              │
                     │ Prediction  │◄─────────────┘
                     │   Market    │
                     └──────┬──────┘
                            │ changesets
                            ▼
                     ┌─────────────┐
                     │  Sequencer  │
                     │  (local)    │
                     └──────┬──────┘
                            │ signed batches
                            ▼
                     ┌─────────────┐
                     │  Validator  │
                     │  (replica)  │
                     └─────────────┘
```

### Watching Bridge Events

With the chain-monitor feature, you can watch for L1 bridge events:

```bash
# Build with chain-monitor feature
cargo build -p prediction-market --features chain-monitor --release

# Start watching (requires anvil running with TestBridge deployed)
./target/release/prediction-market --db market.db watch \
    --ws-url ws://localhost:8545 \
    --bridge 0x5FbDB2315678afecb367f032d93F642f64180aa3 \
    --start-block 1
```

## Testing

```bash
# Run unit tests
cargo test -p prediction-market

# Run with chain monitor feature
cargo test -p prediction-market --features chain-monitor

# Run with verbose output
cargo test -p prediction-market -- --nocapture
```
