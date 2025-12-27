# SyndDB Client FFI Examples

This directory contains Foreign Function Interface (FFI) examples demonstrating how to use the SyndDB client library from different programming languages.

## Overview

The SyndDB client provides a C-compatible FFI interface that can be called from any language with C FFI support. This enables integration with:

- **C/C++** - Direct native integration
- **Python** - Via `ctypes` module (built-in)
- **Node.js** - Via `koffi` package (modern, no compilation needed)
- **Go** - Via `cgo` (standard Go FFI)
- **Ruby** - Via `fiddle` or `ffi` gems
- **And many more...**

## Building the Library

First, build the shared library with FFI support enabled:

```bash
# From the repository root
cargo build --package synddb-client --features ffi --release
```

This creates the shared library at:
- **macOS**: `target/release/libsynddb_client.dylib`
- **Linux**: `target/release/libsynddb_client.so`
- **Windows**: `target/release/synddb_client.dll`

## Available Functions

The FFI interface provides the following functions:

### Core Functions

```c
// Get library version string
const char* synddb_version(void);

// Get last error message (thread-local)
const char* synddb_last_error(void);

// Attach to database with default config
int synddb_attach(
    const char* db_path,
    const char* sequencer_url,
    SyndDBHandle** out_handle
);

// Attach to database with custom config
int synddb_attach_with_config(
    const char* db_path,
    const char* sequencer_url,
    uint64_t flush_interval_ms,
    uint64_t snapshot_interval,
    SyndDBHandle** out_handle
);

// Push pending changesets to sequencer
int synddb_push(SyndDBHandle* handle);

// Create and push a database snapshot
int synddb_snapshot(
    SyndDBHandle* handle,
    size_t* out_size
);

// Detach and cleanup
void synddb_detach(SyndDBHandle* handle);
```

### Error Codes

```c
typedef enum {
    Success = 0,
    InvalidPointer = 1,
    InvalidUtf8 = 2,
    DatabaseError = 3,
    AttachError = 4,
    PublishError = 5,
    SnapshotError = 6,
} SyndDBError;
```

## Language Examples

### C

**Compile:**
```bash
clang -o test_c test.c \
    -L../../../../target/release \
    -lsynddb_client \
    -Wl,-rpath,@loader_path/../../../../target/release
```

**Run:**
```bash
./test_c
```

**Example Code:**
```c
#include <stdio.h>

extern int synddb_attach(
    const char* db_path,
    const char* sequencer_url,
    void** out_handle
);
extern void synddb_detach(void* handle);

int main() {
    void* handle;
    int result = synddb_attach(
        "/tmp/app.db",
        "http://localhost:8433",
        &handle
    );

    if (result == 0) {
        printf("Successfully attached!\\n");
        synddb_detach(handle);
    }
    return result;
}
```

### Python (ctypes)

**No installation required** - `ctypes` is built into Python!

**Run:**
```bash
python3 test.py
```

**Example Code:**
```python
import ctypes
from pathlib import Path

# Load library
lib_path = Path(__file__).parent / "../../../../target/release/libsynddb_client.dylib"
lib = ctypes.CDLL(str(lib_path))

# Define function signatures
lib.synddb_version.restype = ctypes.c_char_p

# Call functions
version = lib.synddb_version()
print(f"Library version: {version.decode('utf-8')}")
```

### Node.js (koffi)

**Install:**
```bash
npm install koffi
```

**Run:**
```bash
node test_koffi.js
```

**Example Code:**
```javascript
const koffi = require('koffi');
const path = require('path');

// Load library
const libPath = path.join(__dirname, '../../../../target/release/libsynddb_client.dylib');
const lib = koffi.load(libPath);

// Define function
const synddb_version = lib.func('synddb_version', 'str', []);

// Call function
const version = synddb_version();
console.log(`Library version: ${version}`);
```

### Go (cgo)

**Example Code:**
```go
package main

/*
#cgo LDFLAGS: -L../../../../target/release -lsynddb_client
#include <stdlib.h>

extern char* synddb_version();
extern int synddb_attach(char* db_path, char* sequencer_url, void** out_handle);
extern void synddb_detach(void* handle);
*/
import "C"
import (
    "fmt"
    "unsafe"
)

func main() {
    version := C.synddb_version()
    fmt.Printf("Library version: %s\\n", C.GoString(version))

    dbPath := C.CString("/tmp/app.db")
    defer C.free(unsafe.Pointer(dbPath))

    sequencerUrl := C.CString("http://localhost:8433")
    defer C.free(unsafe.Pointer(sequencerUrl))

    var handle unsafe.Pointer
    result := C.synddb_attach(dbPath, sequencerUrl, &handle)

    if result == 0 {
        fmt.Println("Successfully attached!")
        C.synddb_detach(handle)
    }
}
```

## Test Results

All FFI examples have been tested and verified:

### ✅ C Test Results
```
=== SyndDB C FFI Test ===

1. Testing synddb_version()...
   Library version: 0.1.0
   ✓ Version check passed

2. Testing error handling (null pointer)...
   Expected error: Null pointer provided
   ✓ Error handling works

3. Testing synddb_attach()...
   ✓ Successfully attached to database

4. Testing synddb_push()...
   ✓ Successfully pushed

5. Testing synddb_snapshot()...
   ✓ Successfully created snapshot (4096 bytes)

6. Testing synddb_detach()...
   ✓ Successfully detached

=== All C FFI tests passed! ===
```

### ✅ Python Test Results
```
=== SyndDB Python ctypes FFI Test ===

1. Testing synddb_version()...
   Library version: 0.1.0
   ✓ Version check passed

2. Testing error handling (null pointer)...
   Expected error: Null pointer provided
   ✓ Error handling works

3. Testing synddb_attach()...
   ✓ Successfully attached to database

4. Testing synddb_push()...
   ✓ Successfully pushed

5. Testing synddb_snapshot()...
   ✓ Successfully created snapshot (4096 bytes)

6. Testing synddb_attach_with_config()...
   ✓ Successfully attached with custom config

7. Testing synddb_detach()...
   ✓ Successfully detached

=== All Python ctypes FFI tests passed! ===
```

### ✅ Node.js Test Results
```
=== SyndDB Node.js koffi FFI Test ===

1. Testing synddb_version()...
   Library version: 0.1.0
   ✓ Version check passed

2. Testing error handling (null pointer)...
   Expected error: Null pointer provided
   ✓ Error handling works

3. Testing synddb_attach()...
   ✓ Successfully attached to database

4. Testing synddb_push()...
   ✓ Successfully pushed

5. Testing synddb_snapshot()...
   ✓ Successfully created snapshot (4096 bytes)

6. Testing synddb_attach_with_config()...
   ✓ Successfully attached with custom config

7. Testing synddb_detach()...
   ✓ Successfully detached

=== All Node.js koffi FFI tests passed! ===
```

## Common Usage Pattern

The typical workflow across all languages is:

1. **Load the library** - Load `libsynddb_client.{dylib,so,dll}`
2. **Define function signatures** - Match the C FFI interface
3. **Call `synddb_attach()`** - Attach to your SQLite database
4. **Use your database normally** - Changesets are automatically captured
5. **Optionally call `synddb_push()`** - For critical transactions
6. **Call `synddb_detach()`** - Cleanup when done

## Error Handling

All functions that return `int` follow this pattern:
- **0 (Success)**: Operation succeeded
- **Non-zero**: Error occurred, call `synddb_last_error()` for details

Error messages are stored in thread-local storage, so they're safe for multi-threaded use.

## Thread Safety

- The FFI interface is thread-safe
- Each thread has its own error message storage
- Multiple threads can call FFI functions concurrently
- Each `SyndDBHandle` manages its own background threads

## Memory Management

- **SyndDBHandle** is opaque - never dereference it directly
- **Always call `synddb_detach()`** to free resources
- **Don't free strings** returned by `synddb_version()` or `synddb_last_error()`
- Error strings are valid until the next FFI call on the same thread

## Platform Notes

### macOS
- Library: `libsynddb_client.dylib`
- Use `-Wl,-rpath,@loader_path/...` for dynamic linking

### Linux
- Library: `libsynddb_client.so`
- Use `-Wl,-rpath,$ORIGIN/...` for dynamic linking
- May need `LD_LIBRARY_PATH` environment variable

### Windows
- Library: `synddb_client.dll`
- Place DLL in same directory as executable
- Or add to PATH environment variable

## Troubleshooting

### Library not found
**macOS/Linux:**
```bash
export LD_LIBRARY_PATH=/path/to/target/release:$LD_LIBRARY_PATH
```

**Windows:**
```cmd
set PATH=C:\path\to\target\release;%PATH%
```

### Symbol not found
Ensure you built with the `ffi` feature:
```bash
cargo build --package synddb-client --features ffi --release
```

### Node.js: ffi-napi compilation errors
Use `koffi` instead - it's modern, faster, and doesn't require compilation:
```bash
npm install koffi
```

## Performance Considerations

- FFI calls have minimal overhead (nanoseconds)
- The client uses background threads for network I/O
- Changesets are batched automatically
- No blocking on database operations

## Additional Resources

- [SyndDB Client Rust Docs](../../src/lib.rs)
- [FFI Implementation](../../src/ffi.rs)
- [Cargo.toml](../../Cargo.toml) - See `crate-type = ["cdylib"]`

## Contributing

To add FFI examples for other languages:

1. Create a new test file (e.g., `test.rb`, `test.swift`)
2. Load the shared library
3. Define function signatures matching `ffi.rs`
4. Call functions and verify results
5. Update this README with your example

## License

Same as the SyndDB project.
