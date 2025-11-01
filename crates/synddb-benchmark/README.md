# SyndDB Benchmark Tool

A high-performance orderbook benchmark tool for testing and developing the SyndDB sidecar. Simulates realistic database workload with configurable throughput and load patterns.

## Features

- **Realistic orderbook operations**: Place orders, cancel orders, execute trades, update balances
- **Multiple load patterns**: Continuous (constant rate) or burst (periodic spikes)
- **Max throughput discovery**: Automatically finds system performance limits
- **Simple mode**: Insert-only operations for maximum throughput stress testing
- **Configurable batching**: Transaction batching for optimal write performance
- **SQLite with WAL mode**: Ready for Session Extension monitoring

## Quick Start

```bash
# Build the benchmark tool
cargo build --package synddb-benchmark --release

# Initialize database and run simulation
cargo run --package synddb-benchmark --release -- init
cargo run --package synddb-benchmark --release -- run --rate 100

# View statistics
cargo run --package synddb-benchmark --release -- stats
```

## Command Reference

### `init` - Initialize Database

Creates the orderbook schema with the following tables:

- **users**: User accounts
- **orders**: Limit and market orders (buy/sell)
- **trades**: Executed trades matching buy and sell orders
- **balances**: User balances per symbol

```bash
cargo run --package synddb-benchmark -- init [--db <path>]
```

### `run` - Run Simulation

Executes orderbook operations based on the selected load pattern. By default, resumes with existing data. Use `--clear` to start fresh.

```bash
cargo run --package synddb-benchmark -- run [OPTIONS]

Options:
  -d, --db <PATH>              Database path [default: orderbook.db]
  -c, --clear                  Clear all existing data before starting [default: resume]
  -p, --pattern <PATTERN>      Load pattern: continuous or burst [default: continuous]
  -r, --rate <RATE>            Operations per second (0 = auto-find max throughput) [default: 100]
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

# Auto-discover maximum throughput (rate=0 enables max mode)
cargo run --package synddb-benchmark --release -- run --rate 0 --simple --batch-size 5000
```

### `stats` - Show Statistics

Display current database statistics including order counts, trade counts, and order status breakdown.

```bash
cargo run --package synddb-benchmark -- stats [--db <path>]
```

### `clear` - Clear Data

Remove all data from tables while preserving the schema.

```bash
cargo run --package synddb-benchmark -- clear [--db <path>]
```

## Database Schema

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

## Operation Distribution

The benchmark simulates realistic orderbook activity:

- **51%** - Place new orders (buy/sell limit orders)
- **15%** - Cancel active orders
- **20%** - Execute trades (match buy and sell orders)
- **14%** - Update balances

## Load Patterns

### Continuous Mode

Maintains a steady rate of operations per second. Useful for:

- Baseline performance testing
- Consistent workload for sidecar development
- Long-running stability tests

```bash
# 100 ops/sec continuously
cargo run --package synddb-benchmark -- run --pattern continuous --rate 100
```

### Burst Mode

Generates periodic bursts of activity with quiet periods between. Useful for:

- Testing batch accumulation in the sidecar
- Simulating real trading activity (quiet periods followed by high activity)
- Testing compression effectiveness on varied workloads

```bash
# 1000 ops every 5 seconds
cargo run --package synddb-benchmark -- run --pattern burst --burst-size 1000 --burst-interval 5
```

## Monitoring

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

## Performance Improvements

The benchmark tool has undergone extensive optimization to maximize SQLite throughput and multi-core utilization. Here are the key improvements and their impact:

### Overview of Optimizations

| Optimization Phase | Simple Mode | Full Mode | Key Improvements |
|-------------------|-------------|-----------|------------------|
| **Baseline** (unoptimized) | ~5,000 ops/sec | ~2,000 ops/sec | Default SQLite configuration |
| **Phase 1: SQLite Tuning** | 8,022 ops/sec | 4,001 ops/sec | PRAGMA optimization, query improvements |
| **Phase 2: Connection Pooling** | 8,022 ops/sec | 4,001 ops/sec | r2d2 pool, atomic counters |
| **Phase 3: Parallel Workers** | **39,997 ops/sec** | **5,064 ops/sec** | Multi-threaded execution |
| **Total Improvement** | **8x faster** | **2.5x faster** | Combined optimizations |

### Phase 1: SQLite Configuration & Query Optimization

**Query Performance (2-4x improvement)**
- **Replaced ORDER BY RANDOM()** with efficient OFFSET-based random selection
  - Previous: Full table scan + sort on every random selection (~30x slower)
  - Current: Index-only scan with random offset (O(log n) performance)
  - Impact: Critical for cancel and trade operations in full mode

- **Added covering indexes**:
  ```sql
  CREATE INDEX idx_orders_status_id ON orders(status, id);
  CREATE INDEX idx_orders_status_side_id ON orders(status, side, id);
  ```
  - Enables index-only queries without table lookups
  - Dramatically improves COUNT(*) and random selection queries

**SQLite PRAGMA Tuning (1.6x improvement)**

Enhanced PRAGMAs for high-throughput workloads:

```rust
// Cache and Memory (4x larger cache)
cache_size = -262144        // 256MB cache (from 64MB)
mmap_size = 30_000_000_000  // 30GB memory-mapped I/O (from 256MB)
temp_store = MEMORY         // Temp tables in RAM

// Concurrency and Locking
busy_timeout = 5000         // Wait 5s for locks (from 0ms immediate fail)
wal_autocheckpoint = 10000  // Reduce checkpoint frequency (from 1000 pages)
journal_size_limit = 64MB   // Control WAL file growth
threads = 4                 // Enable multi-threaded operations (sorting, indexing)

// Prepared Statement Cache
prepared_statement_cache = 128  // Better statement reuse (from 16)
```

**Impact**:
- Simple mode: 5K → 8K ops/sec (**1.6x**)
- Full mode: 2K → 4K ops/sec (**2x**)

### Phase 2: Connection Pooling with r2d2

**Architecture Changes**:
- Migrated from single `rusqlite::Connection` to `r2d2::Pool<SqliteConnectionManager>`
- Pool size dynamically matches worker count (minimum 4 connections)
- Changed `operation_count` to `Arc<AtomicU64>` for thread-safe concurrent updates

**Benefits**:
- Infrastructure ready for parallel execution
- Thread-safe operation counting
- Automatic connection management and reuse
- Graceful handling of connection errors

**Impact**: Minimal direct performance change (foundation for Phase 3)

### Phase 3: Parallel Batch Workers

**Multi-threaded Architecture**:
- Spawns N parallel `tokio::spawn` tasks (workers)
- Each worker has independent connection from pool
- Work distribution via `mpsc::channel` from coordinator
- Concurrent batch processing across CPU cores

**Worker Configuration**:
```bash
# Auto-detect workers (uses half of CPU cores)
cargo run --release -- run --rate 0 --simple

# Specify exact worker count
cargo run --release -- run --rate 0 --workers 5 --simple

# Single-threaded (legacy mode)
cargo run --release -- run --rate 0 --workers 1
```

**Default**: Automatically uses `CPU_CORES / 2` workers to leave headroom for OS and other processes.

**Performance Results** (5 workers on 10-core system):

Simple Mode (inserts only):
- Single-threaded: 8,022 ops/sec
- 5 parallel workers: **39,997 ops/sec** (**5x improvement**)
- CPU utilization: ~60-80% across 5-7 cores

Full Mode (complex operations):
- Single-threaded: 4,001 ops/sec
- 5 parallel workers: **5,064 ops/sec** (initial, degrades with table growth)
- Bottleneck: SQLite's single-writer limitation with complex queries

**Why Full Mode Doesn't Scale Linearly**:

SQLite has a fundamental **single-writer constraint** even in WAL mode:
- Multiple connections can READ simultaneously
- Only **ONE connection can WRITE at a time**
- Complex queries (COUNT, OFFSET, ORDER BY) hold write locks longer
- Multiple workers competing for write lock = contention

**Best Use Cases for Parallel Workers**:
- ✅ Insert-heavy workloads (simple mode): 5-10x improvement
- ✅ Read-heavy workloads: Near-linear scaling with readers
- ⚠️ Write-heavy with complex queries: Limited by SQLite write serialization

### Benchmarking Parallel Workers

Test different worker counts to find optimal configuration for your hardware:

```bash
# Test 1, 2, 4, 8 workers
for workers in 1 2 4 8; do
  echo "Testing $workers workers..."
  cargo run --release -- run --db test.db --clear --rate 0 --workers $workers --simple --duration 10
done
```

**Recommendations by CPU Count**:
- 4 cores: `--workers 2`
- 8 cores: `--workers 4`
- 16 cores: `--workers 8`
- 32+ cores: `--workers 8-12` (diminishing returns beyond SQLite's limits)

### Memory-Mapped I/O (mmap_size)

The 30GB `mmap_size` setting enables memory-mapped I/O for the database file:

**Benefits**:
- Faster reads by mapping file pages directly into process memory
- OS handles page caching automatically
- Reduced system call overhead

**Requirements**:
- Sufficient RAM for active working set
- Modern OS with efficient mmap implementation
- Not beneficial on systems with limited RAM

**Verification**:
```bash
# Check if mmap is being used (run after init)
sqlite3 your_database.db "PRAGMA mmap_size;"
# Should return: 30000000000 (30GB)
```

### Transaction Batching

All modes use transaction batching for optimal throughput. The `--batch-size` flag controls how many operations are grouped per transaction:

**Impact of Batch Size**:
- Without batching: ~100-500 ops/sec (1 transaction per operation)
- Small batches (10-100): ~2,000-5,000 ops/sec
- Medium batches (100-500): ~5,000-10,000 ops/sec (default: 100)
- Large batches (1000-5000): ~10,000-40,000 ops/sec (with parallel workers)

**Trade-offs**:
- Larger batches = Higher throughput, but longer time between commits
- Smaller batches = Lower latency, more frequent commits, lower throughput

### Hardware Recommendations

For maximum performance:

**CPU**:
- More cores = better (diminishing returns beyond 8-12 workers)
- High single-thread performance helps with SQLite's write serialization

**Storage**:
- NVMe SSD strongly recommended for WAL mode
- SATA SSD acceptable for moderate workloads
- HDD will severely bottleneck (avoid for benchmarking)

**Memory**:
- Minimum 8GB for benchmarking
- 16GB+ recommended for large databases
- More RAM enables larger cache_size and benefits mmap_size

### Comparing Configurations

Example benchmark comparing optimizations:

```bash
# Phase 0: Baseline (disable optimizations for comparison)
# Would need to modify code to disable PRAGMAs

# Phase 1: SQLite tuning only (single-threaded)
cargo run --release -- run --db test1.db --clear --rate 0 --workers 1 --simple --duration 30

# Phase 3: Full optimizations (parallel workers)
cargo run --release -- run --db test2.db --clear --rate 0 --workers 5 --simple --duration 30
```

**Expected Results**:
- Phase 1 (single-threaded): ~8,000 ops/sec
- Phase 3 (5 workers): ~40,000 ops/sec
- **Improvement: 5x faster**

## Performance Tuning

The benchmark tool includes several optimizations for high-throughput workloads:

### Transaction Batching

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

### SQLite Optimizations

The benchmark automatically configures SQLite for optimal performance:

- **WAL mode**: Enables concurrent reads during writes
- **256MB cache**: Reduces disk I/O (4x larger than default)
- **Memory temp storage**: Faster temporary table operations
- **30GB mmap**: Memory-mapped I/O for better read performance
- **Multi-threaded operations**: Parallel sorting and indexing (4 threads)
- **Optimized checkpoints**: Less frequent WAL checkpoints (every 10K pages)
- **Prepared statement caching**: Better query plan reuse (128 statements)

These are set automatically in `init`, no configuration needed!

### Choosing Batch Size

- **Smaller batches (10-100)**: Lower latency, more frequent commits, good for real-time applications
- **Medium batches (100-500)**: Balanced throughput and latency (default)
- **Large batches (1000+)**: Maximum throughput, higher latency between commits

### Simple Mode - Maximum Throughput

For stress testing and maximum throughput benchmarking, use `--simple` mode. This bypasses all complex queries (ORDER BY RANDOM(), joins, trade matching) and only performs simple INSERT operations.

```bash
# Simple mode with 50,000 ops/sec
cargo run --package synddb-benchmark --release -- run --simple --rate 50000 --batch-size 5000 --duration 10

# Push to 100,000 ops/sec
cargo run --package synddb-benchmark --release -- run --simple --rate 100000 --batch-size 10000 --duration 10
```

**Performance Comparison:**

| Mode | Single Worker | 5 Parallel Workers | Improvement |
|------|--------------|-------------------|-------------|
| Full mode (all operations) | ~4,000 ops/sec | ~5,000 ops/sec | 1.25x |
| Simple mode (inserts only) | ~8,000 ops/sec | **~40,000 ops/sec** | **5x** |

**When to use Simple Mode:**

- Stress testing SQLite and system performance limits
- Benchmarking raw database write throughput
- Testing sidecar changeset capture under extreme load
- Identifying system bottlenecks (CPU, disk I/O, memory)

**Note:** Simple mode is not representative of realistic orderbook workload, but is useful for finding the maximum performance ceiling of your system.

### Max Throughput Discovery Mode

Set `--rate 0` to automatically discover your system's maximum sustainable throughput using an adaptive algorithm with stability detection.

```bash
# Auto-discover max throughput in simple mode
cargo run --package synddb-benchmark --release -- run --rate 0 --simple --batch-size 5000

# Auto-discover max throughput in full mode
cargo run --package synddb-benchmark --release -- run --rate 0 --batch-size 1000
```

**How it works:**

The algorithm uses **three signals** to detect degradation:

1. **Throughput Achievement**: Measures actual vs. target rate
   - `<90%` = Degraded (triggers backoff and verification)
   - `90-95%` = Marginal (switches to smaller increments)
   - `>95%` = Good (continues doubling)

2. **Stability (Coefficient of Variation)**: Detects performance variance
   - Takes 3 samples of 3 seconds each
   - Calculates mean and CV across samples
   - High CV (>15%) indicates system under stress or resource contention

3. **Adaptive Backoff**: When degradation detected
   - Backs off by 10% from failed rate
   - Runs verification test at backoff rate
   - Reports both best sustained and verified stable rates

**Example output:**
```
Testing 1000 ops/sec
  Sample 1/3: 1972 ops/sec
  Sample 2/3: 1977 ops/sec
  Sample 3/3: 1976 ops/sec
  Mean: 1975 ops/sec (197.5% of target) | Stability: 0.1% CV ✓

Testing 128000 ops/sec
  Mean: 124750 ops/sec (97.5% of target) | Stability: 0.4% CV ✓

Testing 256000 ops/sec
  Mean: 123024 ops/sec (48.1% of target) | Stability: 0.6% CV
  ⚠ Throughput degraded - backing off

Verifying stability at 230400 ops/sec
  Verification 1/3: 123831 ops/sec
  [...]

Maximum Throughput Found:
  Best sustained rate: 124,750 ops/sec
  Verified stable rate: 121,649 ops/sec
```

**Why this approach?**

- **Handles interference**: CV detection catches performance degradation from other processes (e.g., sidecar snapshots, background tasks)
- **Robust**: Multiple samples prevent false positives from transient spikes
- **Adaptive**: Uses large increments when far from limit, small increments when close
- **Conservative**: Backs off and verifies to ensure reported rate is truly sustainable

This is ideal for:
- Finding your system's performance limits under realistic conditions
- Comparing performance across different hardware
- Validating optimizations with statistical confidence
- Benchmarking with other processes running (e.g., sidecar)

## Development

```bash
# Run tests
cargo test --package synddb-benchmark

# Run with logging
RUST_LOG=debug cargo run --package synddb-benchmark -- run --rate 100
```
