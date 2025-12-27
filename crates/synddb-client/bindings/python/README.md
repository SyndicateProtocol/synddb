# SyndDB Python Client

Pure Python FFI wrapper for SyndDB - **no compilation needed!**

## Installation

1. Install Rust (if not already installed):
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```
See https://doc.rust-lang.org/cargo/getting-started/installation.html for more options.

2. Build the Rust library:
```bash
cargo build --package synddb-client --features ffi --release
```

3. Copy `synddb.py` to your project:
```bash
cp bindings/python/synddb.py /path/to/your/project/
```

That's it! No pip install, no build tools required.

## Usage

### Basic Example

```python
from synddb import SyndDB
import sqlite3

# Attach to database
synddb = SyndDB.attach('app.db', 'http://localhost:8433')

# Now use SQLite normally - changesets are automatically captured!
conn = sqlite3.connect('app.db')
conn.execute("CREATE TABLE IF NOT EXISTS users (id INTEGER, name TEXT)")
conn.execute("INSERT INTO users VALUES (?, ?)", (1, 'Alice'))
conn.commit()

# Changes are automatically sent every 1 second

# Clean up when done (optional - Python GC will handle it)
synddb.detach()
```

### With Configuration

```python
from synddb import SyndDB

# Custom publish interval and snapshots
synddb = SyndDB.attach_with_config(
    'app.db',
    'http://localhost:8433',
    send_interval_ms=500,   # Send every 500ms
    snapshot_interval=100       # Snapshot every 100 changesets
)
```

### Manual Sending

Changesets are sent automatically every 1 second. For critical transactions, send immediately:

```python
from synddb import attach

synddb = attach('app.db', 'http://localhost:8433')

# Critical transaction - send immediately after commit
import sqlite3
conn = sqlite3.connect('app.db')
conn.execute("INSERT INTO trades VALUES (?, ?)", (1, 1000000))
conn.commit()
synddb.push()  # Force immediate send
```

**When to call `push()` manually:**
- After critical transactions that must be sent immediately
- Before application shutdown (handled automatically by `detach()`)
- When you need to ensure data is sent before proceeding

**When automatic sending is sufficient:**
- Normal application operations
- High-throughput batch processing

### Context Manager

```python
from synddb import SyndDB

with SyndDB.attach('app.db', 'http://localhost:8433') as synddb:
    import sqlite3
    conn = sqlite3.connect('app.db')
    conn.execute("INSERT INTO users VALUES (?, ?)", (2, 'Bob'))
    conn.commit()
# Automatically detaches and sends on exit
```

## API Reference

### `SyndDB.attach(db_path, sequencer_url)`

Attach to a SQLite database with default configuration.

**Parameters:**
- `db_path` (str): Path to SQLite database file
- `sequencer_url` (str): URL of sequencer TEE

**Returns:** SyndDB instance

### `SyndDB.attach_with_config(db_path, sequencer_url, send_interval_ms=1000, snapshot_interval=100)`

Attach with custom configuration.

**Parameters:**
- `db_path` (str): Path to SQLite database file
- `sequencer_url` (str): URL of sequencer TEE
- `send_interval_ms` (int): Milliseconds between automatic sends (must be > 0, default: 1000)
- `snapshot_interval` (int): Number of changesets between snapshots (must be > 0, default: 100)

**Returns:** SyndDB instance

### `synddb.push()`

Force immediate send of all pending changesets.

### `synddb.snapshot()`

Create and publish a snapshot to the sequencer.

This creates a complete database snapshot (schema + data) and sends it to the sequencer. Use this after schema changes (`CREATE TABLE`, etc.) since DDL is NOT captured in changesets.

**Returns:** Size of snapshot in bytes (int)

**When to use:**
- After `CREATE TABLE`, `ALTER TABLE`, or other DDL statements
- To create periodic recovery checkpoints
- Before major migrations

### `synddb.detach()`

Gracefully shutdown and free resources. Sends any pending changesets.

### `synddb.version()`

Get library version string.

**Returns:** Version string (e.g., "0.1.0")

## Requirements

- Python 3.7+
- SQLite (included with Python)
- libsynddb_client shared library (built from Rust)

## How It Works

This wrapper uses Python's `ctypes` to call the C FFI functions exported by the Rust library. No compilation or build tools required on the Python side - just drop in the `.py` file and go!

The Rust library is compiled once to a shared library (`.so`, `.dylib`, or `.dll`), then all language bindings use the same binary.
