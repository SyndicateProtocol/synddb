# Price Oracle Example

This example demonstrates **custom validator rules** in SyndDB. The key insight:

> **The application maintains API keys; validators check database changesets without needing API access.**

The Python application fetches cryptocurrency prices from multiple sources (CoinGecko, CoinMarketCap) and logs both to the database. The custom validator checks that prices from different sources agree within a configurable tolerance (default: 1%).

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                    Python Price Oracle App                       │
│  - Fetches prices from CoinGecko API (needs API key)            │
│  - Fetches prices from CoinMarketCap API (needs API key)        │
│  - Writes BOTH prices to database (good logging practice)       │
└─────────────────────────────────────────────────────────────────┘
                              │ changesets
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                         Sequencer                                │
│  - Signs and orders changesets                                   │
└─────────────────────────────────────────────────────────────────┘
                              │ signed batches
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│              Custom Validator (price-oracle-validator)           │
│  - Extends synddb-validator with PriceConsistencyRule           │
│  - Queries DB: "Are both prices within 1%?"                     │
│  - Rejects if difference > threshold (NO API keys needed!)      │
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
│   └── api/                  # Price API adapters
│       ├── base.py           # Abstract base class
│       ├── mock.py           # Mock API for testing
│       ├── coingecko.py      # CoinGecko implementation
│       └── coinmarketcap.py  # CoinMarketCap implementation
└── scripts/
    └── dev-env.sh            # Development environment script
```

## Quick Start

### Prerequisites

- Rust toolchain (1.70+)
- Python 3.10+
- OpenSSL (for key generation)

### Run the Demo

```bash
# From the SyndDB root directory
cd examples/price-oracle

# Run with consistent mock APIs (validator accepts)
./scripts/dev-env.sh

# Run with divergent mock APIs (validator rejects!)
./scripts/dev-env.sh --divergent

# Run with real APIs
export COINGECKO_API_KEY="your-key"  # Optional, free tier works
export CMC_API_KEY="your-key"         # Required for CMC
./scripts/dev-env.sh --real
```

### Manual Setup

1. **Build the custom validator:**
   ```bash
   cargo build --release -p price-oracle-validator
   ```

2. **Start the sequencer:**
   ```bash
   SIGNING_KEY=$(openssl rand -hex 32) \
   DATABASE_PATH=./data/sequencer.db \
   ./target/release/synddb-sequencer
   ```

3. **Start the custom validator:**
   ```bash
   SEQUENCER_PUBKEY="<from sequencer logs>" \
   SEQUENCER_URL="http://localhost:8433" \
   DATABASE_PATH=./data/validator.db \
   STATE_DB_PATH=./data/validator_state.db \
   MAX_PRICE_DIFFERENCE_BPS=100 \
   ./target/release/price-oracle-validator
   ```

4. **Run the Python fetcher:**
   ```bash
   cd examples/price-oracle
   python3 -m venv venv
   source venv/bin/activate
   pip install -r app/requirements.txt

   python -m app.fetcher \
     --db ./data/prices.db \
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
