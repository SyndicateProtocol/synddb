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

## 12. Testing Requirements

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

## 13. Error Handling

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

## Summary

The price oracle demonstrates:

1. **Multi-source aggregation** - Fetching from multiple providers
2. **Data consistency validation** - Custom rules ensuring price agreement
3. **SyndDB integration** - Automatic changeset capture and snapshots
4. **Flexible operation modes** - CLI, daemon, and HTTP API
5. **Testing support** - Mock sources with configurable behavior
6. **Optional blockchain bridge** - Bidirectional smart contract communication
