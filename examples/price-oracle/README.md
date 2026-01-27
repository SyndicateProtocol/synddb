# Price Oracle Example

This example demonstrates **custom validator rules** and **bidirectional message passing** in SyndDB. Key concepts:

> **The application maintains API keys; validators check database changesets without needing API access.**

> **Smart contracts can request price updates (pull model) or receive pushed prices (push model) via the Bridge.**

The Python application fetches cryptocurrency prices from multiple sources (CoinGecko, CoinMarketCap) and logs both to the database. The custom validator checks that prices from different sources agree within a configurable tolerance (default: 1%).

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                    Python Price Oracle App                       │
│  - Fetches prices from CoinGecko API (needs API key)            │
│  - Fetches prices from CoinMarketCap API (needs API key)        │
│  - Writes BOTH prices to database (good logging practice)       │
│  - Listens for PriceRequested events (pull model)               │
│  - Pushes prices to chain via Bridge (push model)               │
└─────────────────────────────────────────────────────────────────┘
                              │ changesets
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                         Sequencer                                │
│  - Signs and orders changesets                                   │
│  - Provides message queue API for inbound/outbound messages     │
└─────────────────────────────────────────────────────────────────┘
                              │ signed batches
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│              Custom Validator (price-oracle-validator)           │
│  - Extends synddb-validator with PriceConsistencyRule           │
│  - Queries DB: "Are both prices within 1%?"                     │
│  - Rejects if difference > threshold (NO API keys needed!)      │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                    Anvil (Local Ethereum)                        │
│  ┌─────────────────────────────────────────────────────────────┐│
│  │ Bridge.sol                                                   ││
│  │  - MESSAGE_INITIALIZER_ROLE → Sequencer                     ││
│  │  - Executes validated messages on target contracts          ││
│  └─────────────────────────────────────────────────────────────┘│
│  ┌─────────────────────────────────────────────────────────────┐│
│  │ PriceOracle.sol                                              ││
│  │  - UPDATER_ROLE → Bridge                                    ││
│  │  - updatePrice(): Receive pushed prices                     ││
│  │  - requestPrice(): Emit PriceRequested event                ││
│  └─────────────────────────────────────────────────────────────┘│
└─────────────────────────────────────────────────────────────────┘
```

## Project Structure

```
examples/price-oracle/
├── README.md                 # This file
├── validator/                # Custom Rust validator
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs           # Validator binary
│       └── rules/
│           ├── mod.rs
│           └── price_consistency.rs  # Custom validation rule
├── app/                      # Python application
│   ├── requirements.txt
│   ├── __init__.py
│   ├── main.py               # CLI entry point
│   ├── schema.py             # SQLite schema
│   ├── oracle.py             # Core logic
│   ├── http.py               # Flask HTTP server
│   ├── fetcher.py            # Periodic fetch daemon
│   ├── bridge.py             # Bridge contract interaction
│   ├── chain_monitor.py      # Blockchain event monitoring
│   └── api/                  # Price API adapters
│       ├── base.py           # Abstract base class
│       ├── mock.py           # Mock API for testing
│       ├── coingecko.py      # CoinGecko implementation
│       └── coinmarketcap.py  # CoinMarketCap implementation
├── scripts/
│   └── dev-env.sh            # Development environment script
└── data/                     # Runtime data (gitignored)
    ├── sequencer/            # Sequencer state
    ├── validator/            # Validator state
    ├── app/                  # Application database
    └── contracts/            # Deployed contract addresses
        └── addresses.json    # Contract addresses for current session

contracts/                    # Solidity contracts (project root)
├── src/
│   ├── Bridge.sol            # Cross-chain message bridge
│   └── examples/
│       └── PriceOracle.sol   # On-chain price oracle
└── script/
    ├── DeployLocalDevEnv.s.sol   # Combined local deployment
    └── DeployPriceOracle.s.sol   # Standalone PriceOracle deployment
```

## Quick Start

### Prerequisites

- Rust toolchain (1.70+)
- Python 3.10+
- [Foundry](https://book.getfoundry.sh/getting-started/installation) (for Anvil and contract deployment)
- [just](https://github.com/casey/just) command runner (optional, for easier setup)

### Run the Demo

The easiest way to run the price oracle example is using `just` from the project root:

```bash
# From the SyndDB root directory
just examples::price-oracle
```

This command:
1. Builds the price-oracle-validator binary
2. Sets up the Python virtual environment
3. Deploys contracts to Anvil (starts Anvil if needed)
4. Starts the sequencer and validator
5. Runs the price fetcher with mock data

Alternatively, use the dev environment script for more options:

```bash
cd examples/price-oracle

# Run with consistent mock APIs (validator accepts)
./scripts/dev-env.sh

# Run with divergent mock APIs (validator rejects!)
./scripts/dev-env.sh --divergent

# Run with real APIs
export COINGECKO_API_KEY="your-key"  # Optional, free tier works
export CMC_API_KEY="your-key"         # Required for CMC
./scripts/dev-env.sh --real

# Skip Anvil and contract deployment (off-chain only)
./scripts/dev-env.sh --no-anvil
```

When running with Anvil, the script will:
1. Start Anvil on port 8545
2. Deploy MockWETH, Bridge, and PriceOracle contracts
3. Grant `MESSAGE_INITIALIZER_ROLE` to the sequencer on Bridge
4. Grant `UPDATER_ROLE` to Bridge on PriceOracle
5. Save deployed addresses to `data/contracts/addresses.json`

### Manual Setup

1. **Deploy contracts** (starts Anvil if needed):
   ```bash
   just deploy
   ```

2. **Build the custom validator:**
   ```bash
   cargo build --release -p price-oracle-validator
   ```

3. **Start the sequencer** (key is generated automatically at startup):
   ```bash
   mkdir -p ./data/price-oracle
   BIND_ADDRESS=127.0.0.1:8433 \
   DATABASE_PATH=./data/price-oracle/sequencer.db \
   ./target/release/synddb-sequencer
   ```

4. **Start the custom validator** (fetches sequencer pubkey automatically):
   ```bash
   SEQUENCER_URL="http://localhost:8433" \
   DATABASE_PATH=./data/price-oracle/validator.db \
   STATE_DB_PATH=./data/price-oracle/validator_state.db \
   PENDING_CHANGESETS_DB_PATH=./data/price-oracle/pending_changesets.db \
   MAX_PRICE_DIFFERENCE_BPS=100 \
   ./target/release/price-oracle-validator
   ```

5. **Run the Python fetcher:**
   ```bash
   cd examples/price-oracle
   python3 -m venv venv
   source venv/bin/activate
   pip install -r app/requirements.txt

   python -m app.fetcher \
     --db ../../data/price-oracle/prices.db \
     --sequencer-url http://localhost:8433 \
     --mock \
     --interval 10
   ```

## Python CLI Usage

```bash
# Initialize database
python -m app.main init

# Fetch prices once (mock mode)
python -m app.main --mock fetch

# Fetch with divergent prices (for testing rejection)
python -m app.main --divergent --divergence 5.0 fetch

# Compare prices for an asset
python -m app.main compare BTC

# View recent prices
python -m app.main prices --limit 20

# List tracked assets
python -m app.main assets

# Add a new asset to track
python -m app.main add-asset solana SOL
```

## Custom Validator Configuration

The price oracle validator extends `synddb-validator` with these additional options:

| Option | Environment Variable | Default | Description |
|--------|---------------------|---------|-------------|
| `--max-price-difference-bps` | `MAX_PRICE_DIFFERENCE_BPS` | `100` | Max price difference in basis points (100 = 1%) |
| `--disable-price-check` | `DISABLE_PRICE_CHECK` | `false` | Disable the price consistency rule |

## How the Validation Works

The `PriceConsistencyRule` validates each changeset by:

1. Checking if the `prices` table exists
2. Finding assets with prices from multiple sources
3. For each asset, comparing the latest price from each source
4. Calculating the difference in basis points: `((max - min) / avg) * 10000`
5. **Rejecting** if any asset exceeds the threshold

### Database Schema

```sql
CREATE TABLE prices (
    id INTEGER PRIMARY KEY,
    asset TEXT NOT NULL,           -- 'BTC', 'ETH', etc.
    source TEXT NOT NULL,          -- 'coingecko' or 'coinmarketcap'
    price REAL NOT NULL,           -- Price in USD
    timestamp INTEGER NOT NULL,    -- Unix timestamp
    volume_24h REAL,               -- Optional 24h volume
    market_cap REAL                -- Optional market cap
);
```

## Testing Scenarios

### Scenario 1: Prices Agree (Validator Accepts)

```bash
./scripts/dev-env.sh  # Uses consistent mock APIs
```

The mock APIs return prices that differ by < 1%. The validator accepts all changesets.

### Scenario 2: Prices Diverge (Validator Rejects)

```bash
./scripts/dev-env.sh --divergent --divergence 5.0
```

The mock APIs return prices that differ by ~5%. The validator **rejects** the changesets:

```
WARN Validation rule 'price_consistency' failed at sequence 1:
     Price difference for BTC exceeds threshold:
     CoinGecko=$50000.00, CoinMarketCap=$52500.00,
     difference=487bps (max allowed=100bps)
```

## Running Tests

### Unit Tests (Validation Rule Logic)

Run the Rust unit tests to verify the validation rule works correctly:

```bash
cargo test -p price-oracle-validator

# Output:
# test rules::price_consistency::tests::test_prices_within_tolerance ... ok
# test rules::price_consistency::tests::test_prices_exceed_tolerance ... ok
# test rules::price_consistency::tests::test_multiple_assets ... ok
# ...
```

### Python App Tests (Standalone)

Test the Python app in standalone mode (without SyndDB):

```bash
cd examples/price-oracle
python3 -m venv venv && source venv/bin/activate
pip install -r app/requirements.txt

# Initialize database
python -m app.main init

# Fetch with consistent prices (should work)
python -m app.main fetch --mock

# Compare prices
python -m app.main compare BTC

# Fetch with divergent prices (simulates what validator would reject)
python -m app.main fetch --divergent --divergence 5.0
python -m app.main compare BTC  # Shows >100 bps difference
```

### Known Limitations

**Python FFI Integration**: The current Python FFI bindings create a separate SQLite
connection from the application's connection. This means changes made via Python's
`sqlite3` module aren't captured by SyndDB hooks. For full end-to-end testing with
changesets flowing through the sequencer and validator, use a Rust application
(see the `prediction-market` example).

The Python app demonstrates:
- Price API integration patterns
- Database schema design for multi-source prices
- CLI, HTTP server, and daemon patterns

The Rust validator demonstrates:
- Custom validation rule implementation
- Integration with synddb-validator
- Price consistency checking logic

## Extending This Example

To add your own validation rules:

1. Create a new rule implementing `ValidationRule`:
   ```rust
   use synddb_validator::rules::{ValidationRule, ValidationResult};

   struct MyCustomRule { /* ... */ }

   impl ValidationRule for MyCustomRule {
       fn name(&self) -> &str { "my_custom_rule" }

       fn validate(&self, conn: &Connection, sequence: u64) -> Result<ValidationResult> {
           // Query database and validate
           Ok(ValidationResult::Pass)
       }
   }
   ```

2. Register it with the validator:
   ```rust
   let mut registry = RuleRegistry::new();
   registry.register(Box::new(MyCustomRule::new()));
   registry.register(Box::new(PriceConsistencyRule::new(100)));
   validator.set_rules(registry);
   ```

## Smart Contracts

### PriceOracle.sol

The on-chain price oracle contract supports two message flows:

**Push Model** (off-chain → on-chain):
```solidity
// Off-chain app submits price via Bridge
bridge.initializeAndHandleMessage(
    messageId,
    priceOracleAddress,
    abi.encodeCall(PriceOracle.updatePrice, ("BTC", 50000_00000000, timestamp)),
    sequencerSignature,
    validatorSignatures,
    0  // no native token
);
```

**Pull Model** (on-chain → off-chain → on-chain):
```solidity
// 1. Contract emits request
bytes32 requestId = priceOracle.requestPrice("BTC", 300);  // max 5min old
// Emits: PriceRequested(requestId, "BTC", requester, 300)

// 2. Off-chain app listens, fetches price, responds via Bridge
bridge.initializeAndHandleMessage(
    messageId,
    priceOracleAddress,
    abi.encodeCall(PriceOracle.fulfillPriceRequest, (requestId, "BTC", price, timestamp)),
    ...
);
// Emits: PriceRequestFulfilled(requestId, "BTC", price)
```

### Contract Addresses

After running `dev-env.sh`, deployed addresses are saved to `data/contracts/addresses.json`:

```json
{
    "chainId": 31337,
    "rpcUrl": "http://127.0.0.1:8545",
    "admin": "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
    "sequencer": "0x...",
    "weth": "0x...",
    "bridge": "0x...",
    "priceOracle": "0x..."
}
```

### Role Configuration

| Contract | Role | Granted To | Purpose |
|----------|------|------------|---------|
| Bridge | `MESSAGE_INITIALIZER_ROLE` | Sequencer | Initialize and execute messages |
| Bridge | `DEFAULT_ADMIN_ROLE` | Admin (Anvil account 0) | Manage roles and modules |
| PriceOracle | `UPDATER_ROLE` | Bridge | Call `updatePrice()` and `fulfillPriceRequest()` |
| PriceOracle | `DEFAULT_ADMIN_ROLE` | Admin | Manage roles |

## API Endpoints (HTTP Server)

When running the Flask HTTP server (`python -m app.http`):

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/health` | GET | Health check |
| `/prices` | GET | List recent prices |
| `/prices/<asset>` | GET | Get prices for specific asset |
| `/compare/<asset>` | GET | Compare prices across sources |
| `/fetch` | POST | Trigger a price fetch |
| `/assets` | GET | List tracked assets |
| `/snapshot` | POST | Create SyndDB snapshot |

## License

MIT
