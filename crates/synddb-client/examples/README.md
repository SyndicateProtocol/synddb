# SyndDB Client Examples

This directory contains examples demonstrating how to use the SyndDB client library across multiple languages.

## Quick Start

All examples require a running sequencer. Start it in a separate terminal:

```bash
# From repository root
cargo run --package synddb-sequencer -- --config config/default.yaml
```

## Rust Examples

### Basic Integration

#### [`rust_example.rs`](./rust_example.rs)
**Complexity:** Beginner
**Features:** Basic attachment, automatic publishing, manual publish
**Run:** `cargo run --example rust_example`

The simplest example showing how to integrate SyndDB into a Rust application. Demonstrates:
- One-line integration with `SyndDB::attach()`
- Automatic changeset publishing
- Manual `publish()` calls
- Realistic order book usage pattern

### Snapshot Operations

#### [`snapshot_example.rs`](./snapshot_example.rs)
**Complexity:** Intermediate
**Features:** Snapshot creation, verification, metadata inspection
**Run:** `cargo run --example snapshot_example`

Shows how to create and work with database snapshots:
- Creating snapshots on-demand
- Inspecting snapshot metadata (size, sequence, timestamp)
- Cross-platform snapshot portability
- Proper cleanup patterns

#### [`auto_snapshot_example.rs`](./auto_snapshot_example.rs)
**Complexity:** Intermediate
**Features:** Automatic snapshots, custom configuration
**Run:** `cargo run --example auto_snapshot_example`

Demonstrates automatic snapshot creation based on changeset count:
- Custom `Config` usage
- Interval-based snapshots (every N changesets)
- Configurable publish intervals
- Timing observations

#### [`schema_snapshot_example.rs`](./schema_snapshot_example.rs)
**Complexity:** Advanced
**Features:** Schema change detection, DDL-triggered snapshots
**Run:** `cargo run --example schema_snapshot_example`

Shows how SyndDB automatically creates snapshots when schema changes occur:
- DDL operation detection (ALTER TABLE, CREATE TABLE)
- Immediate snapshot on schema changes
- Schema evolution tracking
- Distinction between DDL and DML operations

## FFI Examples

All FFI examples are located in the [`ffi/`](./ffi/) subdirectory and demonstrate cross-language integration.

### C FFI

#### [`ffi/test.c`](./ffi/test.c)
**Complexity:** Intermediate
**Language:** C
**Compile & Run:**
```bash
cd ffi
clang -o test_c test.c -I../../../../target/release -L../../../../target/release -lsynddb_client
./test_c
```

Complete C FFI example showing:
- All core FFI functions
- Error handling patterns
- Memory management
- Both basic and config-based attachment

### Python FFI

#### [`ffi/test.py`](./ffi/test.py)
**Complexity:** Intermediate
**Language:** Python (ctypes)
**Run:**
```bash
cd ffi
python3 test.py
```

Python ctypes example demonstrating:
- Loading the SyndDB C library
- Ctypes function signatures
- Error code handling
- Proper pointer management

**Note:** Update the library path in the script for your platform:
- macOS: `libsynddb_client.dylib`
- Linux: `libsynddb_client.so`
- Windows: `synddb_client.dll`

### Node.js FFI

#### [`ffi/test_koffi.js`](./ffi/test_koffi.js)
**Complexity:** Intermediate
**Language:** Node.js (koffi)
**Run:**
```bash
cd ffi
npm install  # Install koffi dependency
node test_koffi.js
```

Modern async/await Node.js example showing:
- Koffi library integration
- JavaScript FFI patterns
- Error handling in Node.js
- All core operations

**Note:** Update the library path in the script for your platform.

### Comprehensive FFI Documentation

#### [`ffi/README.md`](./ffi/README.md)

Outstanding comprehensive guide covering:
- C, Python, Node.js, and Go examples
- Complete API reference
- Platform-specific compilation notes
- Troubleshooting guide
- Memory management patterns
- Thread safety considerations

## Work-in-Progress Examples

### Python Native Bindings

#### [`python_example.py`](./python_example.py)
**Status:** TODO - Native Python bindings not yet implemented

This example shows the intended future API for native Python bindings. Currently non-functional.

**For Python integration today, use:** [`ffi/test.py`](./ffi/test.py) (ctypes FFI)

## Prerequisites

### System Requirements
- Rust toolchain (for building examples)
- SQLite 3.x (bundled in builds)
- Running sequencer instance

### Building the Library for FFI Examples

Before running FFI examples, build the client library:

```bash
# From repository root
cargo build --release --package synddb-client
```

The library will be at:
- macOS: `target/release/libsynddb_client.dylib`
- Linux: `target/release/libsynddb_client.so`
- Windows: `target/release/synddb_client.dll`

## Common Patterns

### Database Cleanup

Examples create test databases (`example.db`, `test.db`, etc.). Most examples don't automatically clean up. To remove:

```bash
rm -f example.db test.db snapshot.db orderbook.db *.db-wal *.db-shm
```

### Error Handling

Rust examples use `Result<T>` and `?` for error propagation. FFI examples check error codes:

```c
SyndDBError result = synddb_attach(db_path, url, &handle);
if (result != Success) {
    // Handle error
}
```

### Configuration

Most examples use default configuration. For custom settings:

```rust
let config = Config {
    publish_interval: Duration::from_millis(500),
    snapshot_interval: 5, // Every 5 changesets
};
let synddb = SyndDB::attach_with_config(conn, sequencer_url, config)?;
```

## Getting Help

- **API Documentation:** `cargo doc --package synddb-client --open`
- **Main README:** [`../../README.md`](../../README.md)
- **FFI Guide:** [`ffi/README.md`](./ffi/README.md)
- **Issues:** [GitHub Issues](https://github.com/Syndicate/SyndDB/issues)

## Contributing Examples

Want to add an example? Consider:
- Clear documentation and comments
- Realistic usage patterns
- Error handling demonstrations
- Cleanup code where appropriate
- Header comment with complexity level and features

See existing examples for patterns to follow.
