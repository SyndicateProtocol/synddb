# SyndDB Node.js Client

Pure JavaScript FFI wrapper for SyndDB - **no compilation needed!**

## Installation

1. Build the Rust library:
```bash
cargo build --package synddb-client --features ffi --release
```

2. Install dependencies:
```bash
npm install ffi-napi ref-napi
```

3. Copy `synddb.js` to your project or install from npm (when published):
```bash
# Option 1: Copy file
cp bindings/nodejs/synddb.js /path/to/your/project/

# Option 2: Install from npm (when published)
npm install @synddb/client
```

That's it! No build tools required.

## Usage

### Basic Example

```javascript
const { SyndDB } = require('./synddb');
const Database = require('better-sqlite3');

// Attach to database
const synddb = SyndDB.attach('app.db', 'http://localhost:8433');

// Now use SQLite normally - changesets are automatically captured!
const db = new Database('app.db');
db.prepare('CREATE TABLE IF NOT EXISTS users (id INTEGER, name TEXT)').run();
db.prepare('INSERT INTO users VALUES (?, ?)').run(1, 'Alice');

// Changes are automatically published every 1 second

// Clean up when done (optional - Node.js GC will handle it)
synddb.detach();
```

### With Configuration

```javascript
const { SyndDB } = require('./synddb');

// Custom publish interval and snapshots
const synddb = SyndDB.attachWithConfig(
  'app.db',
  'http://localhost:8433',
  {
    publishIntervalMs: 500,   // Publish every 500ms
    snapshotInterval: 100      // Snapshot every 100 changesets
  }
);
```

### Manual Publishing

Changesets are published automatically every 1 second. For critical transactions, publish immediately:

```javascript
const { attach } = require('./synddb');
const Database = require('better-sqlite3');

const synddb = attach('app.db', 'http://localhost:8433');

// Critical transaction - publish immediately after commit
const db = new Database('app.db');
db.prepare('INSERT INTO trades VALUES (?, ?)').run(1, 1000000);
synddb.publish();  // Don't wait for automatic publish
```

**When to call `publish()` manually:**
- After critical transactions that must be sent immediately
- Before application shutdown (handled automatically by `detach()`)
- When you need to ensure data is sent before proceeding

**When automatic publishing is sufficient:**
- Normal application operations
- High-throughput batch processing

### Using Disposable Pattern (Node.js 20+)

```javascript
const { SyndDB } = require('./synddb');

using synddb = SyndDB.attach('app.db', 'http://localhost:8433');
{
  const Database = require('better-sqlite3');
  const db = new Database('app.db');
  db.prepare('INSERT INTO users VALUES (?, ?)').run(2, 'Bob');
}
// Automatically detaches and publishes on scope exit
```

## API Reference

### `SyndDB.attach(dbPath, sequencerUrl)`

Attach to a SQLite database with default configuration.

**Parameters:**
- `dbPath` (string): Path to SQLite database file
- `sequencerUrl` (string): URL of sequencer TEE

**Returns:** SyndDB instance

### `SyndDB.attachWithConfig(dbPath, sequencerUrl, options)`

Attach with custom configuration.

**Parameters:**
- `dbPath` (string): Path to SQLite database file
- `sequencerUrl` (string): URL of sequencer TEE
- `options` (object):
  - `publishIntervalMs` (number): Milliseconds between automatic publishes (default: 1000)
  - `snapshotInterval` (number): Number of changesets between snapshots (default: 0 = disabled)

**Returns:** SyndDB instance

### `synddb.publish()`

Manually publish all pending changesets immediately.

### `synddb.snapshot()`

Create and publish a snapshot to the sequencer.

This creates a complete database snapshot (schema + data) and sends it to the sequencer. Use this after schema changes (`CREATE TABLE`, etc.) since DDL is NOT captured in changesets.

**Returns:** Size of snapshot in bytes (number)

**When to use:**
- After `CREATE TABLE`, `ALTER TABLE`, or other DDL statements
- To create periodic recovery checkpoints
- Before major migrations

### `synddb.detach()`

Gracefully shutdown and free resources. Publishes any pending changesets.

### `version()`

Get library version string.

**Returns:** Version string (e.g., "0.1.0")

## Requirements

- Node.js 14+
- `ffi-napi` and `ref-napi` packages
- libsynddb_client shared library (built from Rust)

## How It Works

This wrapper uses Node.js's `ffi-napi` to call the C FFI functions exported by the Rust library. No compilation or build tools required on the Node.js side - just install the dependencies and go!

The Rust library is compiled once to a shared library (`.so`, `.dylib`, or `.dll`), then all language bindings use the same binary.

## Troubleshooting

### Library not found error

If you get an error about the library not being found:

1. Make sure you built the Rust library:
   ```bash
   cargo build --package synddb-client --features ffi --release
   ```

2. Set the `LIBSYNDDB_PATH` environment variable:
   ```bash
   export LIBSYNDDB_PATH=/path/to/libsynddb_client.so
   ```

3. Or copy the library to a standard location:
   ```bash
   sudo cp target/release/libsynddb_client.so /usr/local/lib/
   ```
