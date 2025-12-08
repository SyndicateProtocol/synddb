# synddb-client

Lightweight client library for sending SQLite changesets to the SyndDB sequencer.

## Purpose

Runs **in the application's TEE** to capture SQLite changesets and send them to the sequencer TEE (separate VM for key isolation). **Does NOT contain signing keys.**

## Architecture Decision: C FFI (Not PyO3/Neon)

We use **Rust compiled to C ABI** with thin language wrappers (~50 lines each). This gives:
- ✅ Instant install (<10s vs 2-5min compilation)
- ✅ Single binary for all languages
- ✅ No build tools required for users
- ✅ Same approach as SQLite, libcurl, OpenSSL

## Usage

### Rust

```rust
use rusqlite::Connection;
use synddb_client::SyndDB;

fn main() -> Result<()> {
    // Connection must have 'static lifetime (Box::leak is the recommended pattern)
    let conn = Box::leak(Box::new(Connection::open("app.db")?));

    // Single line to enable SyndDB
    let synddb = SyndDB::attach(conn, "https://sequencer:8433")?;

    // Use SQLite normally
    conn.execute("INSERT INTO trades VALUES (?1, ?2)", params![1, 100])?;

    // For transactions, use unchecked_transaction() instead of transaction()
    let tx = conn.unchecked_transaction()?;
    tx.execute("INSERT INTO trades VALUES (?1, ?2)", params![2, 200])?;
    tx.commit()?;

    // Publish changesets to sequencer after committing
    // This is called automatically every 1 second, but can be called manually
    // for critical transactions that need immediate publishing
    synddb.publish()?;

    Ok(())
}
```

> **Note on transactions:** Use `conn.unchecked_transaction()` instead of `conn.transaction()`.
> This is required because SyndDB's session extension holds an immutable borrow of the connection.
> See the [Transactions](#transactions) section for details.

### Python (via C FFI)

```python
import sqlite3
from synddb import attach  # Pure Python wrapper, no compilation

conn = sqlite3.connect('app.db')
attach(conn, sequencer_url='https://sequencer:8433')

# Use SQLite normally
conn.execute("INSERT INTO trades VALUES (?, ?)", (1, 100))
```

### Node.js (via C FFI)

```javascript
const Database = require('better-sqlite3');
const { attach } = require('@synddb/client');  // Pure JS wrapper

const db = new Database('app.db');
attach(db, { sequencerUrl: 'https://sequencer:8433' });

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

> **Thread Safety:** The Session Extension is only accessed from the main thread.
> Background threads only receive `Vec<u8>` bytes through channels - they never access SQLite directly.
> This design eliminates race conditions and ensures safe operation.

## What It Does NOT Do

- ❌ Sign changesets (no keys in application TEE)
- ❌ Publish to DA layers (sequencer's job)
- ❌ Modify application behavior
- ❌ Require schema changes

## Transactions

When using SyndDB, you must use `unchecked_transaction()` instead of `transaction()`:

```rust
// ❌ This won't compile - transaction() requires &mut self
let tx = conn.transaction()?;

// ✅ Use unchecked_transaction() instead
let tx = conn.unchecked_transaction()?;
tx.execute("INSERT INTO ...", params![...])?;
tx.commit()?;
```

**Why?** SyndDB uses SQLite's Session Extension, which requires holding a reference to the `Connection`. Since the session holds this reference, Rust's borrow checker prevents obtaining a mutable borrow (`&mut Connection`) needed by `transaction()`.

**What does "unchecked" mean?** The only difference between `transaction()` and `unchecked_transaction()` is *when* single-transaction semantics are enforced:

| Method | Enforcement | What happens if you nest transactions |
|--------|-------------|---------------------------------------|
| `transaction()` | Compile-time (Rust borrow checker) | Won't compile |
| `unchecked_transaction()` | Runtime (SQLite) | SQLite returns an error |

**There is no functional difference** - both methods create identical transactions with the same isolation and behavior. SQLite only allows one active transaction per connection regardless of which method you use. The "unchecked" simply means Rust won't prevent the mistake at compile time, but SQLite will still catch it at runtime.

## Publishing Changesets

SyndDB automatically publishes changesets every 1 second (configurable). For critical transactions, you can publish immediately:

```rust
// Automatic publishing (default behavior)
// Changesets are published every 1 second in the background

// Manual publishing for critical transactions
let tx = conn.unchecked_transaction()?;
tx.execute("INSERT INTO critical_data VALUES (?1, ?2)", params![...])?;
tx.commit()?;
synddb.publish()?;  // Publish immediately, don't wait for timer
```

**When to call `publish()` manually:**
- After critical transactions that must be sent immediately
- Before application shutdown (handled automatically by `Drop`)
- When you need to ensure data is sent before proceeding

**When automatic publishing is sufficient:**
- Normal application operations
- High-throughput batch processing (publishing after every transaction would be wasteful)

## Configuration

```rust
use synddb_client::{SyndDB, Config};

let config = Config {
    sequencer_url: "https://sequencer:8433".to_string(),
    buffer_size: 100,           // Max changesets before publish
    publish_interval: Duration::from_secs(1),  // Max time before publish
    max_batch_size: 1024 * 1024,  // 1MB
    max_retries: 3,
    request_timeout: Duration::from_secs(10),
};

let _synddb = SyndDB::attach_with_config(&conn, config)?;
```

## Performance

- **Overhead**: ~1-2% CPU overhead for session tracking
- **Memory**: Buffers changesets (configurable, default 1MB)
- **Network**: Batched sends reduce round trips
- **Latency**: Background thread, non-blocking to application

## Security

- Runs in application TEE (same attestation)
- No signing keys stored
- HTTPS + mTLS to sequencer TEE
- Sequencer validates application's TEE attestation

## Cross-Language Support

| Language | Binding Type | Install Time | Status |
|----------|--------------|--------------|--------|
| **Rust** | Native | N/A | ✅ Core implementation |
| **Python** | C FFI (ctypes) | <10s | 🚧 Wrapper in `bindings/python/` |
| **Node.js** | C FFI (ffi-napi) | <10s | 🚧 Wrapper in `bindings/nodejs/` |
| **Go** | C FFI (cgo) | <10s | 🚧 Wrapper in `bindings/go/` |

All non-Rust languages use **pure wrappers** (~50 lines) over `libsynddb.so` - no compilation needed!

## How It Works

We compile Rust to `libsynddb.so` (C ABI), then use trivial wrappers (~50 lines) in each language. Same model as SQLite.

**Why C FFI instead of PyO3/Neon?**
- Simple API (3 functions) doesn't justify PyO3/Neon complexity
- Users get instant install (<10s) vs 2-5min compilation
- Single binary works for all languages
- No build tools required
