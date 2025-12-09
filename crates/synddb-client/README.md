# synddb-client

Lightweight client library for sending SQLite changesets to the SyndDB sequencer.

## Purpose

Runs **in the application's TEE** to capture SQLite changesets and send them to the sequencer TEE (separate VM for key isolation). **Does NOT contain signing keys.**

## Usage

### Rust

```rust
use rusqlite::Connection;
use synddb_client::SyndDB;

fn main() -> Result<()> {
    // Connection must have 'static lifetime (Box::leak is the recommended pattern)
    let conn = Box::leak(Box::new(Connection::open("app.db")?));
    let synddb = SyndDB::attach(conn, "http://sequencer:8433")?;

    // Use SQLite normally - changesets are captured automatically
    conn.execute("INSERT INTO trades VALUES (?1, ?2)", params![1, 100])?;

    // For transactions, use unchecked_transaction() instead of transaction()
    // (required because SyndDB holds a reference to the connection)
    let tx = conn.unchecked_transaction()?;
    tx.execute("INSERT INTO trades VALUES (?1, ?2)", params![2, 200])?;
    tx.commit()?;

    // Changesets publish automatically every 1 second.
    // For critical transactions, publish immediately:
    synddb.publish()?;

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
2. **Registers update hooks** to detect when changes occur
3. **Extracts changesets** when `publish()` is called (automatically every 1 second, or manually)
4. **Buffers changesets** in memory (configurable size)
5. **Background thread** sends batches to sequencer via HTTP
6. **Automatic retries** with exponential backoff
7. **Graceful shutdown** publishes pending changesets

## What It Does NOT Do

- ❌ Sign changesets (no keys in application TEE)
- ❌ Publish to DA layers (sequencer's job)
- ❌ Modify application behavior
- ❌ Require schema changes

## Thread Safety

SyndDB is single-threaded by design because SQLite's Session Extension is not thread-safe. All SQLite operations and SyndDB calls must happen on the same thread that created the instance.

Background threads handle **network I/O only** (sending changesets/snapshots). They receive `Vec<u8>` bytes through channels and never access SQLite directly.

## Configuration

```rust
use synddb_client::{SyndDB, Config};

let config = Config {
    sequencer_url: "http://sequencer:8433".parse().unwrap(),
    buffer_size: 100,                         // Max changesets before publish
    publish_interval: Duration::from_secs(1), // Auto-publish interval
    max_batch_size: 1024 * 1024,              // 1MB
    max_retries: 3,
    request_timeout: Duration::from_secs(10),
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
