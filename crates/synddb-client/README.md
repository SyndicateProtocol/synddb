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

See [FFI_VS_NATIVE_BINDINGS.md](./FFI_VS_NATIVE_BINDINGS.md) for why we chose C FFI over PyO3/Neon.

## Usage

### Rust

```rust
use rusqlite::Connection;
use synddb_client::SyndDB;

fn main() -> Result<()> {
    let conn = Connection::open("app.db")?;

    // Single line to enable SyndDB
    let _synddb = SyndDB::attach(&conn, "https://sequencer:8433")?;

    // Use SQLite normally
    conn.execute("INSERT INTO trades VALUES (?1, ?2)", params![1, 100])?;

    Ok(())
}
```

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
│   ┌─────────────────────────────────┐  │
│   │  App Code                       │  │
│   │  db.execute(...)                │  │
│   └──────────┬──────────────────────┘  │
│              │                          │
│   ┌──────────▼──────────────────────┐  │
│   │  synddb-client Library          │  │
│   │  - Session Extension            │  │
│   │  - Capture changesets           │  │
│   │  - Background sender thread     │  │
│   └──────────┬──────────────────────┘  │
│              │                          │
└──────────────┼──────────────────────────┘
               │
               │ HTTP POST
               ▼
        Sequencer TEE
```

## What It Does

1. **Attaches SQLite Session Extension** to the connection
2. **Registers commit hooks** to capture changesets after each transaction
3. **Buffers changesets** in memory (configurable size)
4. **Background thread** sends batches to sequencer via HTTP
5. **Automatic retries** with exponential backoff
6. **Graceful shutdown** flushes pending changesets

## What It Does NOT Do

- ❌ Sign changesets (no keys in application TEE)
- ❌ Publish to DA layers (sequencer's job)
- ❌ Modify application behavior
- ❌ Require schema changes

## Configuration

```rust
use synddb_client::{SyndDB, Config};

let config = Config {
    sequencer_url: "https://sequencer:8433".to_string(),
    buffer_size: 100,           // Max changesets before flush
    flush_interval: Duration::from_secs(1),  // Max time before flush
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
