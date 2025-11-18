# synddb-client

Lightweight client library for sending SQLite changesets to the SyndDB sequencer.

## Purpose

This library runs **in the application's TEE** and provides minimal integration to capture SQLite changesets and send them to the sequencer TEE. It does NOT contain any signing keys.

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

### Python (via PyO3 bindings)

```python
import sqlite3
from synddb import attach

conn = sqlite3.connect('app.db')
attach(conn, sequencer_url='https://sequencer:8433')

# Use SQLite normally
conn.execute("INSERT INTO trades VALUES (?, ?)", (1, 100))
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

## Language Bindings

- **Rust**: Native (this crate)
- **Python**: PyO3 bindings (TODO)
- **Node.js**: Neon bindings (TODO)
- **Go**: cgo bindings (TODO)
