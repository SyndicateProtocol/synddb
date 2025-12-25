# Prediction Market Example

A simple prediction market demonstrating SyndDB integration. Users can create markets, buy/sell shares, and deposit/withdraw funds via an L1 bridge.

## Architecture

```
┌─────────────┐
│   Anvil     │  Local Ethereum node (L1)
│  (optional) │
└──────┬──────┘
       │ bridge events
       ▼
┌─────────────┐      ┌─────────────┐
│   Chain     │─────►│ Prediction  │  Your application
│  Monitor    │      │   Market    │  (SQLite + business logic)
└─────────────┘      └──────┬──────┘
                            │ changesets
                            ▼
                     ┌─────────────┐
                     │  Sequencer  │  Signs and orders changesets
                     └──────┬──────┘
                            │ signed batches
                            ▼
                     ┌─────────────┐
                     │  Validator  │  Reconstructs state from batches
                     └─────────────┘
```

**The key insight**: Your app just uses SQLite normally. SyndDB captures changes and replicates them.

## Directory Structure

```
prediction-market/
├── src/
│   ├── app.rs           # Main application struct (wraps SyndDB)
│   ├── schema.rs        # SQLite tables
│   ├── market.rs        # Create/resolve markets
│   ├── trading.rs       # Buy/sell shares
│   ├── bridge.rs        # Deposits and withdrawals
│   ├── chain_monitor.rs # L1 event handlers (optional)
│   ├── lib.rs
│   └── main.rs          # CLI
├── scripts/
│   └── dev-env.sh       # Runs the full stack locally
└── Cargo.toml
```

## Quick Start

```bash
# Run everything locally with CLI demo
./examples/prediction-market/scripts/dev-env.sh

# Or run with HTTP server (demonstrates REST API)
./examples/prediction-market/scripts/dev-env.sh --http
```

### CLI Mode

```bash
cargo run -p prediction-market -- init
cargo run -p prediction-market -- create-account alice
cargo run -p prediction-market -- create-market "Will ETH hit 5k?" --resolution-time 1767225600
cargo run -p prediction-market -- buy --account 1 --market 1 --outcome yes --shares 100
cargo run -p prediction-market -- status
```

### HTTP Mode

```bash
# Start the server
cargo run -p prediction-market -- serve --port 8080

# Create account
curl -X POST http://localhost:8080/accounts \
  -H 'Content-Type: application/json' \
  -d '{"name": "alice"}'

# Create market
curl -X POST http://localhost:8080/markets \
  -H 'Content-Type: application/json' \
  -d '{"question": "Will ETH hit 5k?", "resolution_time": 1767225600}'

# Buy shares
curl -X POST http://localhost:8080/markets/1/buy \
  -H 'Content-Type: application/json' \
  -d '{"account_id": 1, "outcome": "yes", "shares": 100}'

# Check status
curl http://localhost:8080/status
```

## SyndDB Integration

The integration is minimal. Here's the core pattern:

```rust
// Open database with replication
let synddb = SyndDB::open("market.db", "http://sequencer:8433")?;
let conn = synddb.connection();

// Use SQLite normally
conn.execute("INSERT INTO accounts (name) VALUES (?)", ["alice"])?;

// Changes are automatically captured and sent to the sequencer
```

That's it. SyndDB hooks into SQLite's session extension to capture changesets transparently.

## Bridge Integration

The example includes L1 bridge support:

- **Deposits**: Chain monitor watches for `Deposit` events, credits user accounts
- **Withdrawals**: Users request withdrawals, validator signs them for the bridge

```bash
# Simulate a deposit (in production, chain monitor does this)
cargo run -p prediction-market -- simulate-deposit \
    --tx-hash 0xabc --from 0x111... --to 0x222... --amount 100000

# Process pending deposits
cargo run -p prediction-market -- process-deposits

# Request a withdrawal
cargo run -p prediction-market -- withdraw --account 1 --amount 50000 --destination 0x123...
```

## Requirements

- Rust toolchain
- Foundry (for dev environment): `curl -L https://foundry.paradigm.xyz | bash && foundryup`

## Testing

```bash
cargo test -p prediction-market
cargo test -p prediction-market --features chain-monitor
```
