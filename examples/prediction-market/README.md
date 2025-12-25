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
cargo run -p synddb-sequencer

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
# Simulate a deposit from L1
cargo run -p prediction-market -- simulate-deposit \
    --tx-hash 0xabc123 \
    --account-name charlie \
    --amount 100000

# Process pending deposits
cargo run -p prediction-market -- process-deposits

# Request a withdrawal
cargo run -p prediction-market -- withdraw --account 1 --amount 50000 --destination 0x1234567890abcdef
```

## Developer Experience Notes

### The Good

1. **Transparent integration**: Once `SyndDB::attach()` is called, all SQLite operations are automatically captured. No changes to business logic needed.

2. **Optional replication**: The app works identically with or without a sequencer URL. Easy to develop/test locally.

3. **Familiar SQLite patterns**: Use standard rusqlite APIs, transactions work as expected.

### Friction Points

1. **`Box::leak` for connection lifetime**: SyndDB requires a `'static` connection lifetime. This is achieved via `Box::leak(Box::new(conn))` which is unusual. Developers may wonder:
   - Is this a memory leak? (Technically yes, but intentional - connection lives for process lifetime)
   - Can I close the connection? (No, and you shouldn't need to)

2. **`unchecked_transaction()` requirement**: Must use `conn.unchecked_transaction()` instead of `conn.transaction()` because SyndDB holds an immutable borrow. This is non-obvious and will cause compile errors that don't clearly explain why.

3. **Publish timing ambiguity**: When should `publish()` be called?
   - After every operation? (High latency, maximum consistency)
   - Periodically? (Batching, but potential data loss on crash)
   - Never? (Rely on 1-second auto-flush)

   Current implementation: caller decides. Maybe SyndDB should offer guidance or helper patterns.

4. **Error handling on sequencer failure**: What happens if the sequencer is unavailable?
   - Operations still succeed locally
   - Changesets are buffered and retried
   - But how does the app know if replication is working?

### Potential Improvements

1. **Connection wrapper**: Could SyndDB provide a wrapper type that hides the `'static` requirement?
   ```rust
   // Instead of Box::leak pattern
   let conn = SyndDBConnection::open("app.db")?;
   ```

2. **Transaction helper**: Provide a transaction wrapper that handles `unchecked_transaction`:
   ```rust
   synddb.transaction(|tx| {
       tx.execute(...)?;
       Ok(())
   })?;
   ```

3. **Publish strategies**: Built-in publish strategies:
   ```rust
   SyndDB::attach_with_config(conn, Config {
       publish_strategy: PublishStrategy::AfterCommit, // or ::Manual, ::Timer(Duration)
       ...
   })?;
   ```

4. **Health/status API**: Expose replication status:
   ```rust
   if synddb.is_healthy() { ... }
   let stats = synddb.stats(); // pending changesets, last publish time, etc.
   ```

## Schema

```sql
-- Markets (binary YES/NO outcomes)
CREATE TABLE markets (
    id INTEGER PRIMARY KEY,
    question TEXT NOT NULL,
    outcome TEXT DEFAULT 'unresolved',  -- 'yes', 'no', or 'unresolved'
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

-- Chain monitor tables
CREATE TABLE inbound_deposits (...);
CREATE TABLE outbound_withdrawals (...);
```

## Testing

```bash
# Run unit tests
cargo test -p prediction-market

# Run with verbose output
cargo test -p prediction-market -- --nocapture
```
