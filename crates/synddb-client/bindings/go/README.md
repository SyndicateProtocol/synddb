# SyndDB Go Client

Go bindings for SyndDB using CGO.

## Installation

1. Build the Rust library:
```bash
cargo build --package synddb-client --features ffi --release
```

2. Set up your Go project:
```bash
go mod init your-project
```

3. Copy the binding or use as a module:
```bash
# Option 1: Copy the file
cp bindings/go/synddb.go your-project/

# Option 2: When published as a Go module
go get github.com/syndicate/synddb-go
```

4. Set library path for linking:
```bash
# macOS
export CGO_LDFLAGS="-L/path/to/target/release"
export DYLD_LIBRARY_PATH="/path/to/target/release:$DYLD_LIBRARY_PATH"

# Linux
export CGO_LDFLAGS="-L/path/to/target/release"
export LD_LIBRARY_PATH="/path/to/target/release:$LD_LIBRARY_PATH"
```

## Usage

### Basic Example

```go
package main

import (
    "log"
    "github.com/syndicate/synddb-go"
)

func main() {
    // Attach to database
    handle, err := synddb.Attach("/tmp/app.db", "http://localhost:8433")
    if err != nil {
        log.Fatal(err)
    }
    defer handle.Detach()

    // Create schema (auto-snapshots on DDL)
    err = handle.ExecuteBatch(`
        CREATE TABLE IF NOT EXISTS users (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL
        )
    `)
    if err != nil {
        log.Fatal(err)
    }

    // Insert data - changes are captured and sent
    rows, err := handle.Execute("INSERT INTO users (name) VALUES ('Alice')")
    if err != nil {
        log.Fatal(err)
    }
    log.Printf("Inserted %d row(s)", rows)

    // Force immediate push for critical data
    err = handle.Push()
    if err != nil {
        log.Fatal(err)
    }
}
```

### With Configuration

```go
config := synddb.Config{
    PushIntervalMs:   500,  // Push every 500ms
    SnapshotInterval: 100,  // Auto-snapshot every 100 changesets
}

handle, err := synddb.AttachWithConfig("/tmp/app.db", "http://localhost:8433", config)
if err != nil {
    log.Fatal(err)
}
defer handle.Detach()
```

### Transactions

```go
// Start transaction
err := handle.Begin()
if err != nil {
    log.Fatal(err)
}

// Execute operations
_, err = handle.Execute("INSERT INTO users (name) VALUES ('Bob')")
if err != nil {
    handle.Rollback()
    log.Fatal(err)
}

_, err = handle.Execute("INSERT INTO users (name) VALUES ('Charlie')")
if err != nil {
    handle.Rollback()
    log.Fatal(err)
}

// Commit
err = handle.Commit()
if err != nil {
    log.Fatal(err)
}
```

## API Reference

### Functions

#### `Attach(dbPath, sequencerURL string) (*Handle, error)`

Attach to a SQLite database file with default configuration.

#### `AttachWithConfig(dbPath, sequencerURL string, config Config) (*Handle, error)`

Attach with custom configuration.

#### `Version() string`

Get library version string.

#### `LastError() string`

Get the last error message.

### Handle Methods

#### `Detach()`

Gracefully disconnect and free resources.

#### `Push() error`

Force immediate push of pending changesets.

#### `Snapshot() (int, error)`

Create and publish a database snapshot. Returns size in bytes.

#### `Execute(sql string) (int64, error)`

Execute a single SQL statement. Returns rows affected.

#### `ExecuteBatch(sql string) error`

Execute multiple SQL statements. Auto-snapshots on DDL.

#### `Begin() error`

Start a transaction.

#### `Commit() error`

Commit the current transaction.

#### `Rollback() error`

Rollback the current transaction.

## Requirements

- Go 1.18+
- CGO enabled
- libsynddb_client shared library

## Platform Notes

### macOS

```bash
export DYLD_LIBRARY_PATH="/path/to/target/release:$DYLD_LIBRARY_PATH"
```

### Linux

```bash
export LD_LIBRARY_PATH="/path/to/target/release:$LD_LIBRARY_PATH"
```

Or install the library system-wide:
```bash
sudo cp target/release/libsynddb_client.so /usr/local/lib/
sudo ldconfig
```

## Troubleshooting

### "library not found"

Make sure the library path is set:
```bash
# Check library exists
ls -la target/release/libsynddb_client.*

# Set appropriate environment variable
export DYLD_LIBRARY_PATH="$(pwd)/target/release"  # macOS
export LD_LIBRARY_PATH="$(pwd)/target/release"    # Linux
```

### CGO errors

Ensure CGO is enabled:
```bash
export CGO_ENABLED=1
```
