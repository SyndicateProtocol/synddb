# Price Oracle Example - Feature Specification

This document specifies the features required for a price oracle application built on SyndDB. The oracle aggregates cryptocurrency prices from multiple sources, stores them in SQLite, and validates price consistency across sources.

## Overview

A price oracle fetches asset prices from multiple data providers, stores them locally, and publishes updates through SyndDB. A custom validator rule ensures price consistency across sources before accepting changesets.

---

## 1. Asset Management

### Tracked Assets

The oracle maintains a list of assets to monitor. Each asset has:

| Field | Description |
|-------|-------------|
| `symbol` | API-compatible identifier (e.g., `bitcoin`, `ethereum`) |
| `display_name` | Human-readable ticker (e.g., `BTC`, `ETH`) |
| `active` | Whether the asset is currently being fetched |

### Default Assets

- Bitcoin (BTC)
- Ethereum (ETH)

### Supported Assets

Additional assets the oracle should be capable of tracking:

- Solana (SOL)
- Cardano (ADA)
- Polkadot (DOT)
- Ripple (XRP)
- Dogecoin (DOGE)
- Avalanche (AVAX)
- Chainlink (LINK)
- Polygon (MATIC)

### Asset Operations

| Operation | Description |
|-----------|-------------|
| Add asset | Register a new asset for tracking |
| Remove asset | Deactivate an asset (stop fetching) |
| List assets | Show all tracked assets and their status |

---

## 2. Price Data Sources

### Required Sources

The oracle must support at least two independent price sources to enable cross-validation.

#### CoinGecko

- Free tier available (10-50 calls/minute)
- Optional API key for higher rate limits
- Batch queries supported
- Returns: price, 24h volume, market cap

#### CoinMarketCap

- Requires API key (333 calls/day on free tier)
- Uses ticker symbols vs CoinGecko IDs
- Returns: price, 24h volume, market cap

### Mock Sources (Testing)

For development and testing, mock sources must be available:

| Mode | Behavior |
|------|----------|
| Consistent | Returns prices within configurable tolerance (e.g., 0.5% variance) |
| Divergent | Returns prices exceeding tolerance (e.g., 5% difference) |

Mock sources should support:
- Deterministic output via RNG seed
- Configurable variance percentage
- Price caching within fetch cycles for consistency

---

## 3. Price Storage

### Price Records

Each price fetch stores:

| Field | Type | Description |
|-------|------|-------------|
| `asset` | string | Asset symbol |
| `source` | string | Data source identifier |
| `price` | decimal | USD price |
| `timestamp` | integer | Unix timestamp |
| `volume_24h` | decimal | 24-hour trading volume (optional) |
| `market_cap` | decimal | Market capitalization (optional) |

### Indexes

- `(asset, timestamp DESC)` - Query latest prices per asset
- `(source)` - Filter by data source

### Price Snapshots

Aggregated snapshots for historical analysis:

| Field | Description |
|-------|-------------|
| `asset` | Asset symbol |
| `avg_price` | Average across sources |
| `min_price` | Minimum price |
| `max_price` | Maximum price |
| `price_diff_bps` | Difference in basis points |
| `sources_count` | Number of contributing sources |
| `timestamp` | Snapshot time |

---

## 4. Price Fetching

### Fetch Behavior

- Query all active assets from all configured sources
- Store each price record individually
- Handle API errors gracefully (log and continue)
- Support rate limiting per source

### Daemon Mode

Continuous operation with:

| Option | Description |
|--------|-------------|
| `interval` | Seconds between fetch cycles (default: 60) |
| `snapshot_interval` | Create snapshot every N fetch cycles |

### Fetch Modes

| Mode | Description |
|------|-------------|
| Real | Use actual API endpoints with configured keys |
| Mock | Use mock sources with consistent prices |
| Divergent | Use mock sources with divergent prices |

---

## 5. Price Comparison

### Single Asset Comparison

For any asset, compute:

| Metric | Description |
|--------|-------------|
| `avg_price` | Average across all sources |
| `min_price` | Lowest reported price |
| `max_price` | Highest reported price |
| `difference_bps` | `((max - min) / avg) * 10000` |
| `difference_pct` | `((max - min) / avg) * 100` |

### Latest Price Selection

When comparing, use only the most recent price from each source for a given asset.

---

## 6. Validation Rules

### Price Consistency Rule

A custom SyndDB validator rule that rejects changesets where prices from different sources diverge beyond a threshold.

#### Validation Logic

1. After changeset application, query the prices table
2. For each asset with prices from multiple sources:
   - Get the latest price per source
   - Calculate basis point difference: `((max - min) / avg) * 10000`
3. Reject if any asset exceeds `max_price_difference_bps`

#### Configuration

| Option | Default | Description |
|--------|---------|-------------|
| `max_price_difference_bps` | 100 | Maximum allowed difference (100 = 1%) |
| `disable_price_check` | false | Skip validation entirely |

#### Validation Outcomes

| Scenario | Result |
|----------|--------|
| Prices within tolerance | Accept |
| Prices exceed tolerance | Reject with detailed reason |
| Single source only | Not applicable (skip) |
| No prices table | Not applicable (skip) |

---

## 7. SyndDB Integration

### Changeset Capture

- All database writes go through a monitored connection
- Changesets are automatically captured
- Schema changes (DDL) trigger automatic snapshots

### Manual Snapshots

- CLI command to create on-demand snapshots
- Daemon mode with scheduled snapshot intervals
- Returns snapshot size in bytes

### Sequencer Communication

- Publish changesets to configured sequencer URL
- Snapshot publishing for initial state

---

## 8. HTTP API

### Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/health` | Health check |
| `GET` | `/prices` | List recent prices (query: `asset`, `limit`) |
| `GET` | `/prices/<asset>` | Get prices for specific asset |
| `GET` | `/prices/<asset>/history` | Get price history in time range |
| `GET` | `/prices/<asset>/stats` | Get statistical summary over time window |
| `GET` | `/prices/<asset>/ohlc` | Get OHLC candlestick data |
| `GET` | `/compare/<asset>` | Compare prices across sources |
| `POST` | `/fetch` | Trigger manual price fetch |
| `GET` | `/assets` | List tracked assets |
| `POST` | `/snapshot` | Create database snapshot |

### Response Formats

#### Price Record
```json
{
  "asset": "BTC",
  "source": "coingecko",
  "price": 42150.25,
  "timestamp": 1703520000,
  "volume_24h": 15000000000,
  "market_cap": 825000000000
}
```

#### Comparison Response
```json
{
  "asset": "BTC",
  "avg_price": 42100.00,
  "min_price": 42000.00,
  "max_price": 42200.00,
  "difference_bps": 47,
  "difference_pct": 0.47,
  "sources": ["coingecko", "coinmarketcap"]
}
```

#### Price Stats Response
```json
{
  "asset": "BTC",
  "window_seconds": 3600,
  "count": 120,
  "avg_price": 42150.50,
  "min_price": 41800.00,
  "max_price": 42500.00,
  "stddev": 185.32,
  "volatility_pct": 0.44,
  "from_timestamp": 1703516400,
  "to_timestamp": 1703520000
}
```

#### OHLC Candle Response
```json
{
  "asset": "BTC",
  "interval": "1h",
  "open_price": 42100.00,
  "high_price": 42500.00,
  "low_price": 41800.00,
  "close_price": 42350.00,
  "volume": 1500000000,
  "timestamp": 1703516400
}
```

---

## 9. CLI Commands

### Core Commands

| Command | Description |
|---------|-------------|
| `init` | Initialize database schema |
| `fetch` | Fetch prices from all sources |
| `prices` | Show recent prices |
| `compare <asset>` | Compare prices across sources |
| `assets` | List tracked assets |
| `add-asset <symbol> <name>` | Add new asset |
| `snapshot` | Create database snapshot |

### Daemon Commands

| Command | Description |
|---------|-------------|
| `run-daemon` | Run continuous fetcher with periodic updates |

### Global Options

| Option | Description |
|--------|-------------|
| `-v, --verbose` | Enable verbose logging |
| `--db <path>` | Database path |
| `--sequencer-url <url>` | SyndDB sequencer URL |
| `--mock` | Use mock data sources |
| `--divergent` | Use divergent mock sources |

---

## 10. Bridge Integration

For blockchain-connected oracles, support bidirectional messaging via the Bridge validator.

### Message Format

Price updates use the `updatePrice(string,uint256,uint256)` message type:

| Parameter | Type | Description |
|-----------|------|-------------|
| `asset` | string | Asset symbol (e.g., "bitcoin", "ethereum") |
| `priceScaled` | uint256 | Price scaled by 10^18 (1 USD = 1e18) |
| `timestamp` | uint256 | Unix timestamp of the price observation |

### Calldata Encoding

Calldata is ABI-encoded using proper Ethereum ABI encoding:
- 4-byte function selector: keccak256("updatePrice(string,uint256,uint256)")[:4]
- Followed by ABI-encoded parameters

### Metadata Schema

Messages include metadata for validator verification. Schema: `schemas/update-price.schema.json`

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `reason` | string | Yes | Must be "price_update" |
| `asset` | string | Yes | Asset symbol matching calldata |
| `price_scaled` | string | Yes | Price as string matching calldata |
| `timestamp` | integer | Yes | Timestamp matching calldata |
| `source` | string | Yes | Source identifier (e.g., "price-oracle") |
| `sources_count` | integer | No | Number of aggregated sources |
| `price_diff_bps` | integer | No | Price divergence in basis points |

### Validator Invariants

The Bridge validator can be configured with custom invariants for price validation:

1. **PriceMetadataConsistencyInvariant**: Verifies metadata matches calldata
   - Decodes calldata using ABI
   - Compares asset, price, and timestamp with metadata
   - Rejects if any value mismatches

2. **PriceDivergenceInvariant**: Enforces price consistency across sources
   - Reads `price_diff_bps` from metadata
   - Rejects if divergence exceeds threshold (default: 100 bps = 1%)

### PriceOracle Contract

The target contract `PriceOracle.sol` implements:
- `updatePrice(string,uint256,uint256)` - Updates on-chain price
- `getPrice(string)` - Returns price, timestamp, and block number
- Stale price rejection (older timestamps not accepted)
- Future timestamp rejection (max 5 minutes ahead)
- Zero price rejection

### Bridge Commands

| Command | Description |
|---------|-------------|
| `push-prices` | Push current prices to contract |
| `process-requests` | Process pending price requests |
| `message-stats` | Show message queue statistics |
| `watch` | Monitor contract events |

---

## 11. Configuration

### Environment Variables

| Variable | Description |
|----------|-------------|
| `DATABASE_PATH` | SQLite database location |
| `SEQUENCER_URL` | SyndDB sequencer endpoint |
| `COINGECKO_API_KEY` | CoinGecko API key (optional) |
| `CMC_API_KEY` | CoinMarketCap API key (required for CMC) |
| `MAX_PRICE_DIFFERENCE_BPS` | Validator tolerance threshold |
| `DISABLE_PRICE_CHECK` | Disable validation rule |

---

## 12. End-to-End Testing

The price oracle includes comprehensive E2E tests that verify the full flow from price fetching through validator to smart contract.

### Prerequisites

1. **Anvil** - Local Ethereum node
2. **Foundry** - For contract deployment and interaction
3. **Rust** - For building the validator

### Setup Steps

```bash
# 1. Start Anvil
anvil --host 127.0.0.1 --port 8545 --chain-id 31337

# 2. Deploy base Bridge contracts (from project root)
./scripts/setup-e2e-test.sh

# 3. Deploy PriceOracle and register domain
./examples/price-oracle/scripts/setup-e2e.sh

# 4. Start the validator with price oracle invariants
source .e2e-test-env
source examples/price-oracle/.e2e-env
cargo run -p synddb-bridge-validator -- \
    --bridge-address $BRIDGE_ADDRESS \
    --bridge-chain-id 31337 \
    --rpc-url $RPC_URL \
    --private-key $VALIDATOR_PRIVATE_KEY \
    --http-port 8080 \
    --enable-price-oracle-invariants \
    --price-divergence-max-bps 100
```

### Test Suites

| Test File | Description | Requires Validator |
|-----------|-------------|--------------------|
| `test_oracle.py` | Unit tests for price fetching and comparison | No |
| `test_bridge_integration.py` | Calldata encoding and message structure | No |
| `test_e2e_bridge.py` | Contract deployment and registration | No |
| `test_validator_e2e.py` | Full validator message submission flow | Yes |

### Running Tests

```bash
# Without validator (unit tests and contract tests)
source examples/price-oracle/.e2e-env
cd examples/price-oracle
source .venv/bin/activate
pytest tests/test_oracle.py tests/test_bridge_integration.py tests/test_e2e_bridge.py -v

# With validator (full E2E flow)
# Start validator first, then:
pytest tests/test_validator_e2e.py -v -s
```

### Validator Configuration Options

| Option | Env Var | Description |
|--------|---------|-------------|
| `--enable-price-oracle-invariants` | `ENABLE_PRICE_ORACLE_INVARIANTS` | Enable metadata/calldata consistency checks |
| `--price-divergence-max-bps` | `PRICE_DIVERGENCE_MAX_BPS` | Maximum allowed price divergence (100 = 1%) |

---

## 13. Mock Testing

### Mock Mode

- Deterministic prices for reproducible tests
- Configurable divergence percentage
- Seed-based randomization

### Test Scenarios

| Scenario | Expected Outcome |
|----------|------------------|
| Consistent prices within tolerance | Validator accepts |
| Divergent prices exceeding tolerance | Validator rejects |
| Single source only | Validation skipped |
| API failure | Graceful error handling, continue with available sources |
| Multiple assets, one fails validation | Entire changeset rejected |

### Development Environment

Script to orchestrate all components:

| Mode | Description |
|------|-------------|
| Consistent | Mock prices that pass validation |
| Divergent | Mock prices that fail validation |
| Real | Actual API calls (requires keys) |

---

## 14. Error Handling

### API Errors

- Log failures and continue with available sources
- Retry on next fetch cycle
- Never fail entire fetch due to single source failure

### Validation Errors

- Return detailed rejection reason including:
  - Which asset failed
  - Actual difference in basis points
  - Threshold that was exceeded

### Database Errors

- Schema initialization should be idempotent
- Handle concurrent access gracefully

---

## 15. Complex Queries (SQLite vs Message-Passing)

This section documents the analytical query capabilities that demonstrate SQLite's strengths compared to pure message-passing architectures.

### SQLite Advantages

SQLite enables complex analytical queries with a single statement:

| Query Type | SQLite | Message-Passing |
|------------|--------|-----------------|
| Historical stats (avg, min, max, stddev) | One SQL query | Must index events off-chain |
| OHLC candlestick data | GROUP BY with time buckets | Requires off-chain indexer |
| Time-range queries | Indexed scan | Query each block/event |
| Ad-hoc analysis | Any SQL query | Must pre-plan indexes |

### Available Analytical Endpoints

#### Price Statistics (`GET /prices/<asset>/stats`)

Returns statistical summary over a configurable time window:
- Supported windows: `1m`, `5m`, `15m`, `1h`, `4h`, `1d`
- Computes: count, avg, min, max, stddev, volatility percentage

**SQL Query Pattern:**
```sql
SELECT
    COUNT(*) as count,
    AVG(price) as avg_price,
    MIN(price) as min_price,
    MAX(price) as max_price,
    AVG(price * price) as avg_sq  -- For stddev calculation
FROM prices
WHERE asset = ? AND timestamp >= ?
```

#### OHLC Candlesticks (`GET /prices/<asset>/ohlc`)

Returns candlestick data for charting:
- Supported intervals: `1m`, `5m`, `15m`, `1h`, `4h`, `1d`
- Computes: open, high, low, close prices per interval

**SQL Query Pattern:**
```sql
WITH bucketed AS (
    SELECT price, timestamp, (timestamp / interval) * interval as bucket
    FROM prices WHERE asset = ?
)
SELECT bucket, MIN(price), MAX(price),
       FIRST_VALUE(price) as open, LAST_VALUE(price) as close
FROM bucketed GROUP BY bucket
```

### Message-Passing Equivalent

In a pure message-passing system (no local database), these queries would require:

1. **Off-chain indexer**: A separate service that watches on-chain events and builds queryable state
2. **Repeated contract calls**: Query the contract for each data point (expensive, slow)
3. **Pre-computed views**: Store aggregations on-chain (high gas costs)

This tradeoff is fundamental: SQLite provides rich query capabilities at the cost of decentralization, while message-passing provides consensus guarantees at the cost of query flexibility.

---

## 16. Failure Handling (Message-Passing Challenges)

This section documents the failure handling patterns required for reliable message-passing, contrasting with SQLite's simpler transactional model.

### Error Classification

The Bridge validator returns error codes that clients must classify:

| Error Type | Retryable | Example Codes |
|------------|-----------|---------------|
| Transient | Yes | `BRIDGE_CONNECTION_FAILED`, `TIMEOUT`, `INTERNAL_ERROR` |
| Validation | No | `INVALID_NONCE`, `SCHEMA_VALIDATION_FAILED`, `INVARIANT_VIOLATED` |
| Authorization | No | `APP_NOT_AUTHORIZED`, `MESSAGE_TYPE_NOT_REGISTERED` |
| State | No | `REPLAY_DETECTED`, `TIMESTAMP_EXPIRED` |

### Retry Logic

The `BridgeClient` implements automatic retry with exponential backoff:

```python
class BridgeClient:
    def __init__(
        self,
        validator_url: str,
        domain: str,
        max_retries: int = 3,      # Retry limit
        retry_delay: float = 1.0,  # Initial delay (doubles each attempt)
        timeout: float = 30.0,     # Request timeout
    ):
        ...

    async def push_price_with_retry(
        self, asset: str, price: float, **kwargs
    ) -> PushResult:
        """Push with automatic retry for transient errors."""
```

### Status Polling

Messages progress through 9 stages on-chain. Clients must poll to track progress:

| Stage | Status | Description |
|-------|--------|-------------|
| 0 | `not_initialized` | Message not yet submitted |
| 1 | `pending` | Awaiting validator signatures |
| 2 | `ready` | Signature threshold met |
| 3-5 | `executing` | Execution in progress |
| 6 | `completed` | Successfully executed |
| 7 | `failed` | Execution failed |
| 8 | `expired` | Message expired before execution |

```python
# Wait for message to complete with timeout
status = await client.wait_for_completion(
    message_id,
    timeout=60.0,
    poll_interval=2.0,
)
if status.is_success:
    print("Message executed successfully")
```

### SQLite Comparison

| Aspect | SQLite | Message-Passing |
|--------|--------|-----------------|
| Transaction result | Immediate (commit/rollback) | Must poll for status |
| Retry logic | Not needed (ACID) | Client must implement |
| Error handling | Exception-based | Error code classification |
| Status tracking | N/A | 9-stage lifecycle |
| Timeout handling | Connection timeout only | Request + polling timeout |

### Failure Scenarios

| Scenario | Expected Behavior |
|----------|-------------------|
| Validator down | Retry with backoff, fail after max_retries |
| Invalid domain | Immediate failure, no retry |
| Nonce conflict | Immediate failure, client increments nonce |
| Price divergence | Immediate failure (invariant violated) |
| Network timeout | Retry with backoff |
| Message pending | Poll until completed/failed/expired |

---

## Summary

The price oracle demonstrates:

1. **Multi-source aggregation** - Fetching from multiple providers
2. **Data consistency validation** - Custom rules ensuring price agreement
3. **SyndDB integration** - Automatic changeset capture and snapshots
4. **Flexible operation modes** - CLI, daemon, and HTTP API
5. **Testing support** - Mock sources with configurable behavior
6. **Optional blockchain bridge** - Bidirectional smart contract communication
7. **Complex analytical queries** - Demonstrating SQLite's query capabilities
8. **Failure handling patterns** - Retry, polling, and error classification for message-passing
