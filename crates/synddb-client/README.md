# synddb-client

Lightweight client library for pushing SQLite changesets to the SyndDB sequencer.

## Purpose

Runs **in the application's TEE** to capture SQLite changesets and push them to the sequencer TEE (separate VM for key isolation). **Does NOT contain signing keys.**

## Usage

### Rust

```rust
use rusqlite::Connection;
use synddb_client::SyndDB;

fn main() -> Result<()> {
    // Connection requires 'static lifetime
    let conn = Box::leak(Box::new(Connection::open("app.db")?));
    let synddb = SyndDB::attach(conn, "http://sequencer:8433")?;

    // Use SQLite normally - changesets are captured automatically
    conn.execute("INSERT INTO trades VALUES (?1, ?2)", params![1, 100])?;

    // For transactions, use unchecked_transaction() instead of transaction()
    // (required because SyndDB holds a reference to the connection)
    let tx = conn.unchecked_transaction()?;
    tx.execute("INSERT INTO trades VALUES (?1, ?2)", params![2, 200])?;
    tx.commit()?;

    // Optionally force immediate push (auto-pushes every second)
    synddb.push()?;

    Ok(())
}
```

### Python (via C FFI)

```python
import sqlite3
from synddb import attach  # Pure Python wrapper, no compilation

conn = sqlite3.connect('app.db')
attach(conn, sequencer_url='http://sequencer:8433')

# Use SQLite normally
conn.execute("INSERT INTO trades VALUES (?, ?)", (1, 100))
```

### Node.js (via C FFI)

```javascript
const Database = require('better-sqlite3');
const { attach } = require('@synddb/client');  // Pure JS wrapper

const db = new Database('app.db');
attach(db, { sequencerUrl: 'http://sequencer:8433' });

// Use SQLite normally
db.prepare("INSERT INTO trades VALUES (?, ?)").run(1, 100);
```

## Architecture

```
┌─────────────────────────────────────────┐
│   Application Process (in TEE)          │
│                                         │
│   ┌─────────────────────────────────┐   │
│   │  App Code                       │   │
│   │  db.execute(...)                │   │
│   └──────────┬──────────────────────┘   │
│              │                          │
│   ┌──────────▼──────────────────────┐   │
│   │  synddb-client Library          │   │
│   │  - Session Extension            │   │
│   │  - Capture changesets           │   │
│   │  - Background sender thread     │   │
│   └──────────┬──────────────────────┘   │
│              │                          │
└──────────────┼──────────────────────────┘
               │
               │ HTTP POST
               ▼
        Sequencer TEE
```

## What It Does

1. **Attaches SQLite Session Extension** to the connection
2. **Captures changesets** automatically via update hooks
3. **Background thread** sends batches to sequencer via HTTP
4. **Automatic retries** with exponential backoff
5. **Graceful shutdown** sends any remaining pending changesets

## Pushing Changesets

Changesets are automatically pushed every second (configurable via `push_interval`). Use `push()` to force immediate push for low-latency or high-value changes:

```rust
// After a transaction or batch of operations
let tx = conn.unchecked_transaction()?;
tx.execute("INSERT INTO orders ...", params![...])?;
tx.execute("UPDATE balances ...", params![...])?;
tx.commit()?;

// Force immediate push (optional - auto-pushes every second)
synddb.push()?;
```

**When to call `push()`:**
- For low-latency changes that shouldn't wait for the next timer tick
- For high-value operations where immediate confirmation is important
- Before graceful shutdown (also called automatically on `Drop`)

Changesets are also automatically pushed when `SyndDB` is dropped (graceful shutdown).

## What It Does NOT Do

- ❌ Sign changesets (no keys in application TEE)
- ❌ Publish to storage layers (sequencer's job)
- ❌ Modify application behavior
- ❌ Require schema changes

## Thread Safety

SyndDB is single-threaded by design because SQLite's Session Extension is not thread-safe. All SQLite operations and SyndDB calls must happen on the same thread that created the instance.

Background threads handle **network I/O only** (sending changesets/snapshots). They receive `Vec<u8>` bytes through channels and never access SQLite directly.

## Why `'static` Lifetime?

`SyndDB::attach()` requires `&'static Connection` because the SQLite Session Extension is stored in thread-local storage, which requires `'static` bounds. We use `Box::leak` to satisfy this:

```rust
let conn = Box::leak(Box::new(Connection::open("app.db")?));
```

**Trade-offs:**
- The `Connection` is intentionally leaked (never dropped by Rust)
- SQLite cleanup (closing file handles, WAL checkpoint) happens at process exit
- This is acceptable for typical single-connection-per-process usage

**Note:** `SyndDB` itself is dropped normally and performs graceful shutdown (sending pending changesets, joining background threads).

**Manual Connection cleanup:** If you need to explicitly close the Connection (e.g., to flush WAL), you can reclaim ownership after shutting down SyndDB:

```rust
// Shutdown SyndDB first
synddb.shutdown()?;

// Then unsafely reclaim and close the connection
unsafe {
    let boxed = Box::from_raw(conn as *const Connection as *mut Connection);
    let _ = boxed.close();
}
```

## Configuration

```rust
use synddb_client::{SyndDB, Config};

let config = Config {
    sequencer_url: "http://sequencer:8433".parse().unwrap(),
    buffer_size: 100,              // Max changesets to buffer
    max_batch_size: 1024 * 1024,   // 1MB max batch size
    max_retries: 3,                // Retry count for failed sends
    snapshot_interval: 100,        // Snapshot every 100 changesets (0 to disable)
    ..Default::default()
};

let synddb = SyndDB::attach_with_config(conn, config)?;
```

## Performance

- **Overhead**: ~1-2% CPU for session tracking
- **Memory**: Buffers changesets (configurable, default 1MB)
- **Network**: Batched sends reduce round trips
- **Latency**: Background thread, non-blocking to application

## Cross-Language Support

We use **Rust compiled to C ABI** with thin language wrappers (~50 lines each):

| Language | Binding Type | Install Time | Status |
|----------|--------------|--------------|--------|
| **Rust** | Native | N/A | ✅ Core implementation |
| **Python** | C FFI (ctypes) | <10s | 🚧 Wrapper in `bindings/python/` |
| **Node.js** | C FFI (ffi-napi) | <10s | 🚧 Wrapper in `bindings/nodejs/` |
| **Go** | C FFI (cgo) | <10s | 🚧 Wrapper in `bindings/go/` |

**Why C FFI instead of PyO3/Neon?**
- Instant install (<10s vs 2-5min compilation)
- Single binary for all languages
- No build tools required for users
- Same approach as SQLite, libcurl, OpenSSL
