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

Executes orderbook operations based on the selected load pattern. By default, resumes with existing data. Use `--clean` to start fresh.

```bash
cargo run --package synddb-benchmark -- run [OPTIONS]

Options:
  -d, --db <PATH>              Database path [default: orderbook.db]
  --clean                      Clean all existing data before starting [default: resume]
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

# Start fresh, cleaning existing data
cargo run --package synddb-benchmark -- run --clean --rate 100

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

The benchmark tool has undergone extensive performance analysis and optimization. Through rigorous testing, we identified that **extra indexes significantly hurt write performance**, while **PRAGMA tuning and parallel workers provide substantial gains**.

### Overview of Optimizations

| Optimization Phase | Simple Mode (Max Throughput) | Key Improvements |
|-------------------|------------------------------|------------------|
| **Baseline** (original) | 124,533 ops/sec | Original config with basic PRAGMAs |
| **Current** (optimized) | **125,662 ops/sec** | PRAGMA tuning + connection pooling + parallel workers |
| **Total Improvement** | **+1%** | Marginal improvement, but insights gained |

**Key Finding**: The original configuration was already well-tuned. Our "optimizations" initially **decreased performance by 28%** due to extra indexes before we corrected course.

### What We Learned

**❌ What Hurt Performance:**

1. **Covering Indexes on Write-Heavy Workloads**
   - Added indexes: `idx_orders_status_id` and `idx_orders_status_side_id`
   - **Impact**: Reduced throughput by **12%** (119,622 → 104,881 ops/sec)
   - **Why**: Every INSERT must update ALL indexes. For insert-only workloads, extra indexes are pure overhead.
   - **Lesson**: Only add indexes that directly support your query patterns. Don't blindly add "covering" indexes.

**✅ What Helped Performance:**

1. **PRAGMA Tuning (+1% improvement)**
   ```rust
   // Cache and Memory (4x larger cache)
   cache_size = -262144        // 256MB cache (from 64MB)
   mmap_size = 30_000_000_000  // 30GB memory-mapped I/O (from 256MB)
   temp_store = MEMORY         // Temp tables in RAM

   // Concurrency and Locking
   busy_timeout = 5000         // Wait 5s for locks (from 0ms immediate fail)
   wal_autocheckpoint = 10000  // Reduce checkpoint frequency (from 1000 pages)
   journal_size_limit = 64MB   // Control WAL file growth

   // Prepared Statement Cache
   prepared_statement_cache = 128  // Better statement reuse (from 16)
   ```
   - Larger cache helps with concurrent access patterns
   - Higher mmap_size reduces I/O overhead
   - Relaxed checkpointing reduces write stalls

2. **Connection Pooling (infrastructure only)**
   - Migrated from single `rusqlite::Connection` to `r2d2::Pool<SqliteConnectionManager>`
   - Pool size dynamically matches worker count (minimum 4 connections)
   - Changed `operation_count` to `Arc<AtomicU64>` for thread-safe concurrent updates
   - No direct performance change, but enables parallel execution

3. **Parallel Batch Workers (enables multi-core utilization)**
   - Spawns N parallel `tokio::spawn` tasks (workers)
   - Each worker has independent connection from pool
   - Work distribution via `mpsc::channel` from coordinator
   - Concurrent batch processing across CPU cores
   - **Impact**: Enables better CPU utilization when bottleneck is not SQLite itself

### Understanding SQLite's Single-Writer Limitation

Even with all optimizations, SQLite has a fundamental constraint:

**The Single-Writer Bottleneck:**
- Multiple connections can READ simultaneously
- Only **ONE connection can WRITE at a time** (even in WAL mode)
- Write transactions are serialized at the SQLite level
- Parallel workers help with **task distribution**, not concurrent writes

**Why This Matters:**
- Simple mode (insert-only): Each transaction is quick, so multiple workers can rotate through the write lock efficiently
- Full mode (complex queries): COUNT(*) and OFFSET queries hold locks longer, increasing contention
- **Bottleneck shifts from CPU to SQLite's write serialization**

**Best Use Cases for Parallel Workers:**
- ✅ Insert-heavy workloads with small transactions
- ✅ Mixed read-write workloads (readers don't block each other)
- ⚠️ Complex write transactions (limited by single-writer constraint)

### Worker Configuration
```bash
# Auto-detect workers (uses half of CPU cores)
cargo run --release -- run --rate 0 --simple

# Specify exact worker count
cargo run --release -- run --rate 0 --workers 5 --simple

# Single-threaded (legacy mode)
cargo run --release -- run --rate 0 --workers 1
```

**Default**: Automatically uses `CPU_CORES / 2` workers to leave headroom for OS and other processes.

**Performance Results** (5 workers on 10-core M1 Max):
- Peak achieved: **125,662 ops/sec** (simple mode)
- Sustained stable: **122,202 ops/sec** (verified over 15s)
- CPU utilization: Better distribution across cores compared to single-threaded

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

### Benchmarking Your System

To find the maximum throughput for your hardware:

```bash
# Run max throughput discovery on simple mode
cargo run --release -- run --db test.db --clean --rate 0 --simple --duration 90

# Test different worker counts
for workers in 1 2 4 8; do
  echo "Testing $workers workers..."
  cargo run --release -- run --db test_$workers.db --clean --rate 0 --workers $workers --simple --duration 30
done
```

**What to Expect** (M1 Max 10-core):
- Max throughput: ~125K ops/sec (5 workers)
- Single-threaded: ~120K ops/sec (SQLite is already very fast!)
- Multi-core helps with task distribution but gains are marginal due to SQLite's single-writer constraint

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
  Peak achieved rate: 124,750 ops/sec
  Sustained stable rate: 121,649 ops/sec (verified over 15s)
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
