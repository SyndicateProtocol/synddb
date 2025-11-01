# SyndDB - High-Performance Blockchain Database

SyndDB enables developers to build high-performance blockchain applications using **any programming language** with SQLite bindings. The sidecar automatically captures and publishes SQL operations for verification and replication.

## Documentation Guide

- **[SPEC.md](SPEC.md)** - Complete technical specification covering architecture, terminology, and design decisions
- **[PLAN_SIDECAR.md](PLAN_SIDECAR.md)** - Sidecar implementation plan (Session Extension monitoring, batching, publishing)

## Repository Structure

```
SyndDB/
├── crates/
│   ├── synddb-benchmark/    # Orderbook benchmark tool for development
│   └── synddb-sidecar/      # Sidecar listener (coming soon)
├── SPEC.md                  # Full specification
├── PLAN_SIDECAR.md         # Sidecar architecture plan
└── README.md               # This file
```

## Orderbook Benchmark Tool

A simple benchmarking tool that simulates orderbook operations to generate realistic database workload for sidecar development.

### Features

- **Realistic orderbook operations**: Place orders, cancel orders, execute trades, update balances
- **Multiple load patterns**: Continuous (constant rate) or burst (periodic spikes)
- **Configurable throughput**: Control operations per second
- **SQLite with WAL mode**: Ready for Session Extension monitoring
- **Comprehensive statistics**: Track orders, trades, and database activity

CLAUDE: This quick start is duplicative of the later orderbook simulation code. Keep the quick start to just the build and basic commands, and remove the later duplication.

### Quick Start

```bash
# Build the benchmark tool
cargo build --package synddb-benchmark --release

# Initialize the database with orderbook schema
cargo run --package synddb-benchmark --release -- init

# Run continuous load at 100 ops/sec
cargo run --package synddb-benchmark --release -- run --rate 100

# Run burst mode (1000 ops every 5 seconds)
cargo run --package synddb-benchmark --release -- run --pattern burst --burst-size 1000 --burst-interval 5

# Show database statistics
cargo run --package synddb-benchmark --release -- stats

# Clear all data
cargo run --package synddb-benchmark --release -- clear
```

### Command Reference

#### `init` - Initialize Database

Creates the orderbook schema with the following tables:

- **users**: User accounts
- **orders**: Limit and market orders (buy/sell)
- **trades**: Executed trades matching buy and sell orders
- **balances**: User balances per symbol

```bash
cargo run --package synddb-benchmark -- init [--db <path>]
```

#### `run` - Run Simulation

Executes orderbook operations based on the selected load pattern. By default, resumes with existing data. Use `--clear` to start fresh.

```bash
cargo run --package synddb-benchmark -- run [OPTIONS]

Options:
  -d, --db <PATH>              Database path [default: orderbook.db]
  -c, --clear                  Clear all existing data before starting [default: resume]
  -p, --pattern <PATTERN>      Load pattern: continuous or burst [default: continuous]
  -r, --rate <RATE>            Operations per second (continuous mode) [default: 100]
  -t, --duration <SECONDS>     Duration in seconds (0 = run forever) [default: 0]
  -b, --burst-size <SIZE>      Burst size (burst mode) [default: 1000]
  -i, --burst-interval <SECS>  Pause between bursts (burst mode) [default: 5]
  --batch-size <SIZE>          Transaction batch size (higher = faster) [default: 100]
  --simple                     Simple mode: only insert orders (no queries, much faster) [default: false]
```

**Examples:**

```bash
# Run at 500 ops/sec for 60 seconds (resumes with existing data)
cargo run --package synddb-benchmark -- run --rate 500 --duration 60

# Start fresh, clearing existing data
cargo run --package synddb-benchmark -- run --clear --rate 100

# Run burst mode with 5000 ops every 10 seconds
cargo run --package synddb-benchmark -- run --pattern burst --burst-size 5000 --burst-interval 10

# Run continuously at max speed (useful for stress testing)
cargo run --package synddb-benchmark -- run --rate 10000

# Simple mode: maximum throughput with only inserts (50,000-100,000+ ops/sec)
cargo run --package synddb-benchmark --release -- run --simple --rate 100000 --batch-size 10000
```

#### `stats` - Show Statistics

Display current database statistics including order counts, trade counts, and order status breakdown.

```bash
cargo run --package synddb-benchmark -- stats [--db <path>]
```

#### `clear` - Clear Data

Remove all data from tables while preserving the schema.

```bash
cargo run --package synddb-benchmark -- clear [--db <path>]
```

### Database Schema

The benchmark creates a realistic orderbook schema:

```sql
-- Users table
CREATE TABLE users (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    username TEXT NOT NULL UNIQUE,
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
);

-- Orders table
CREATE TABLE orders (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL,
    symbol TEXT NOT NULL,
    side TEXT NOT NULL CHECK(side IN ('buy', 'sell')),
    order_type TEXT NOT NULL CHECK(order_type IN ('limit', 'market')),
    price INTEGER,
    quantity INTEGER NOT NULL,
    filled_quantity INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'active' CHECK(status IN ('active', 'filled', 'cancelled', 'partial')),
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    updated_at INTEGER NOT NULL DEFAULT (unixepoch()),
    FOREIGN KEY (user_id) REFERENCES users(id)
);

-- Trades table
CREATE TABLE trades (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    buy_order_id INTEGER NOT NULL,
    sell_order_id INTEGER NOT NULL,
    symbol TEXT NOT NULL,
    price INTEGER NOT NULL,
    quantity INTEGER NOT NULL,
    buyer_id INTEGER NOT NULL,
    seller_id INTEGER NOT NULL,
    executed_at INTEGER NOT NULL DEFAULT (unixepoch()),
    FOREIGN KEY (buy_order_id) REFERENCES orders(id),
    FOREIGN KEY (sell_order_id) REFERENCES orders(id)
);

-- Balances table
CREATE TABLE balances (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL,
    symbol TEXT NOT NULL,
    amount INTEGER NOT NULL DEFAULT 0,
    locked INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL DEFAULT (unixepoch()),
    FOREIGN KEY (user_id) REFERENCES users(id),
    UNIQUE(user_id, symbol)
);
```

### Operation Distribution

The benchmark simulates realistic orderbook activity:

- **51%** - Place new orders (buy/sell limit orders)
- **15%** - Cancel active orders
- **20%** - Execute trades (match buy and sell orders)
- **14%** - Update balances

### Load Patterns

#### Continuous Mode

Maintains a steady rate of operations per second. Useful for:

- Baseline performance testing
- Consistent workload for sidecar development
- Long-running stability tests

```bash
# 100 ops/sec continuously
cargo run --package synddb-benchmark -- run --pattern continuous --rate 100
```

#### Burst Mode

Generates periodic bursts of activity with quiet periods between. Useful for:

- Testing batch accumulation in the sidecar
- Simulating real trading activity (quiet periods followed by high activity)
- Testing compression effectiveness on varied workloads

```bash
# 1000 ops every 5 seconds
cargo run --package synddb-benchmark -- run --pattern burst --burst-size 1000 --burst-interval 5
```

### Monitoring

The benchmark logs progress every 5 seconds:

```
[INFO] Operations: 500 | Elapsed: 5.0s | Rate: 100.0 ops/sec
[INFO] Operations: 1000 | Elapsed: 10.0s | Rate: 100.0 ops/sec
```

Use the `stats` command to see detailed database state:

```bash
cargo run --package synddb-benchmark -- stats

# Output:
# [INFO] === Orderbook Statistics ===
# [INFO] Users:           100
# [INFO] Orders:          5234 total
# [INFO]   - Active:      1523
# [INFO]   - Filled:      2891
# [INFO]   - Cancelled:   820
# [INFO] Trades:          2103
```

### Performance Tuning

The benchmark tool includes several optimizations for high-throughput workloads:

#### Transaction Batching

Use `--batch-size` to group multiple operations into single transactions. This dramatically improves write performance:

```bash
# Default batch size (100 ops/transaction)
cargo run --package synddb-benchmark -- run --rate 10000

# Larger batches for maximum throughput (1000 ops/transaction)
cargo run --package synddb-benchmark -- run --rate 10000 --batch-size 1000

# Small batches for lower latency (10 ops/transaction)
cargo run --package synddb-benchmark -- run --rate 1000 --batch-size 10
```

**Performance Impact:**

- Without batching: ~100-500 ops/sec
- With batching (default 100): ~2,000-5,000 ops/sec (10-50x improvement)
- With large batches (1000+): Can achieve even higher throughput

#### SQLite Optimizations

The benchmark automatically configures SQLite for optimal performance:

- **WAL mode**: Enables concurrent reads during writes
- **64MB cache**: Reduces disk I/O
- **Memory temp storage**: Faster temporary table operations
- **256MB mmap**: Memory-mapped I/O for better performance

These are set automatically in `init`, no configuration needed!

#### Choosing Batch Size

- **Smaller batches (10-100)**: Lower latency, more frequent commits, good for real-time applications
- **Medium batches (100-500)**: Balanced throughput and latency (default)
- **Large batches (1000+)**: Maximum throughput, higher latency between commits

#### Simple Mode - Maximum Throughput

For stress testing and maximum throughput benchmarking, use `--simple` mode. This bypasses all complex queries (ORDER BY RANDOM(), joins, trade matching) and only performs simple INSERT operations.

```bash
# Simple mode with 50,000 ops/sec
cargo run --package synddb-benchmark --release -- run --simple --rate 50000 --batch-size 5000 --duration 10

# Push to 100,000 ops/sec
cargo run --package synddb-benchmark --release -- run --simple --rate 100000 --batch-size 10000 --duration 10
```

**Performance Comparison:**

- Full mode (all operations): ~2,000-5,000 ops/sec
- Simple mode (inserts only): ~50,000-100,000+ ops/sec (10-20x improvement)

**When to use Simple Mode:**

- Stress testing SQLite and system performance limits
- Benchmarking raw database write throughput
- Testing sidecar changeset capture under extreme load
- Identifying system bottlenecks (CPU, disk I/O, memory)

**Note:** Simple mode is not representative of realistic orderbook workload, but is useful for finding the maximum performance ceiling of your system.

## SyndDB Sidecar (Coming Soon)

The sidecar will implement the architecture described in `PLAN_SIDECAR.md`:

1. **Session Monitor** - Attach to SQLite via Session Extension
2. **Batcher** - Accumulate changesets and create periodic snapshots
3. **Attestor** - Compress and sign batches with TEE-protected keys
4. **Publisher** - Publish to multiple DA layers (Celestia, EigenDA, IPFS, Arweave)

### Development Workflow

1. Run the benchmark tool to generate database activity
2. Develop/test the sidecar against the live database
3. Monitor changeset capture, batching, and publishing
4. Iterate on sidecar implementation

```bash
# Terminal 1: Run benchmark
cargo run --package synddb-benchmark -- run --rate 100

# Terminal 2: Run sidecar (once implemented)
cargo run --package synddb-sidecar -- --db orderbook.db
```

## Key Features

- **Language Agnostic**: Works with any language that has SQLite bindings
- **High Performance**: Sub-millisecond writes, high throughput
- **Deterministic Replication**: Session Extension changesets for validators
- **Automatic Publishing**: Sidecar handles all DA layer interaction
- **Zero Code Changes**: Drop-in solution for existing SQLite applications

## Requirements

- Rust 1.90.0 or later
- SQLite 3.x (bundled with rusqlite)

## Development

```bash
# Build all crates
cargo build --workspace

# Run tests
cargo test --workspace

# Run benchmark with logging
RUST_LOG=debug cargo run --package synddb-benchmark -- run --rate 100
```

## License

MIT License - see LICENSE file for details
