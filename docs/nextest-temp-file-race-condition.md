# Nextest Temp File Race Condition

## Problem

When using [cargo-nextest](https://nexte.st/) for parallel test execution, tests that create temporary files with "unique" paths based on static counters can collide.

### Root Cause

Nextest runs each test in a **separate process** for isolation. This means:

1. Static variables (like `AtomicU64` counters) reset to their initial value for each test
2. Thread IDs are typically `ThreadId(1)` (main thread) in each process
3. Two tests running in parallel will generate the **same** temp file path

### Symptoms

Tests fail intermittently with errors like:
- `"attempt to write a readonly database"` (SQLite)
- `"disk I/O error"`
- File permission errors
- Corruption when two processes write to the same file

### Example: Broken Code

```rust
fn create_temp_db() -> PathBuf {
    // BROKEN: Counter resets per-process in nextest
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let thread_id = std::thread::current().id();

    // Two parallel tests will both generate:
    // /tmp/test_db_0_ThreadId(1).db
    std::env::temp_dir().join(format!("test_db_{id}_{thread_id:?}.db"))
}
```

## Solution

Include the **process ID** in the temp file path. Process IDs are unique across parallel test processes.

### Fixed Code

```rust
fn create_temp_db() -> PathBuf {
    // Counter still useful for multiple calls within same test
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let pid = std::process::id();  // Unique per nextest process

    // Each test process gets unique paths:
    // /tmp/test_db_12345_0.db, /tmp/test_db_12346_0.db, etc.
    std::env::temp_dir().join(format!("test_db_{pid}_{id}.db"))
}
```

## Alternative Solutions

### 1. Use the `tempfile` crate (Recommended for complex cases)

```rust
use tempfile::NamedTempFile;

fn create_temp_db() -> NamedTempFile {
    NamedTempFile::new().unwrap()
}
```

### 2. Use UUIDs

```rust
use uuid::Uuid;

fn create_temp_db() -> PathBuf {
    let id = Uuid::new_v4();
    std::env::temp_dir().join(format!("test_db_{id}.db"))
}
```

### 3. Use timestamp + random component

```rust
use std::time::{SystemTime, UNIX_EPOCH};

fn create_temp_db() -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let pid = std::process::id();
    std::env::temp_dir().join(format!("test_db_{pid}_{timestamp}.db"))
}
```

## When This Applies

This issue affects any test that:
- Creates temporary files with "unique" paths
- Uses static counters or thread IDs for uniqueness
- Runs under nextest (or any test runner that uses process-per-test)

Standard `cargo test` runs tests as threads within a single process, so this issue may not appear until you switch to nextest.

## References

- [Nextest execution model](https://nexte.st/book/how-it-works.html)
- Original fix: `crates/synddb-validator/src/apply/applier.rs`
