# Cross-Language Strategy for SyndDB Client

## The Question

**How do we support Python, Node.js, Go, Rust, etc. with minimal code duplication?**

## Answer: Rust Core + C FFI (The SQLite Model)

This is the same approach SQLite uses, and it's the simplest for cross-language support.

### Architecture

```
┌──────────────────────────────────────────────┐
│   libsynddb.so (Rust compiled to C ABI)      │
│   - SQLite Session Extension integration     │
│   - HTTP client for sequencer                │
│   - Background thread management             │
│   - Single implementation, compiled once     │
└──────────────────────────────────────────────┘
                    ▲
        C FFI (stable ABI)
                    │
    ┌───────┬───────┼───────┬───────┐
    │       │       │       │       │
┌───▼──┐ ┌──▼──┐ ┌──▼───┐ ┌▼────┐ ┌▼────┐
│ Rust │ │Python│ │Node.js│ │ Go │ │ C++ │
│native│ │ctypes│ │ffi-napi│ │cgo│ │direct│
└──────┘ └──────┘ └───────┘ └────┘ └─────┘
```

## Implementation Complexity

### Option A: Native per Language ❌
```
Rust:     1000 lines (Session Ext, HTTP, threading)
Python:   1000 lines (duplicate logic)
Node.js:  1000 lines (duplicate logic)
Go:       1000 lines (duplicate logic)
TOTAL:    4000 lines + maintenance nightmare
```

### Option B: Rust + Language-Specific Bindings ⚠️
```
Rust core:     1000 lines
PyO3 binding:   200 lines (Rust proc macros)
Neon binding:   200 lines (Rust + Node build)
cgo binding:    200 lines (Go wrapper)
TOTAL:         1600 lines + complex builds
```

### Option C: Rust + C FFI + Thin Wrappers ✅ (RECOMMENDED)
```
Rust core:     1000 lines
C FFI layer:    100 lines (simple exports)
Python wrapper:  50 lines (pure Python, no build)
Node.js wrapper: 50 lines (pure JS, no build)
Go wrapper:      50 lines (pure Go, no build)
TOTAL:         1250 lines + trivial maintenance
```

## Why Option C is Simplest

### 1. Single Binary Distribution

**Build once, use everywhere:**
```bash
# Build Rust to shared library with C ABI
cargo build --release --lib

# Produces: libsynddb.so (Linux), libsynddb.dylib (macOS), synddb.dll (Windows)
```

**Users install:**
```bash
# System-wide
sudo cp target/release/libsynddb.so /usr/local/lib/

# Or package-specific
pip install synddb  # includes .so in wheel
npm install @synddb/client  # includes .so in package
go get github.com/syndicate/synddb-go  # includes .so
```

### 2. Zero Compilation in Target Languages

**Python binding (50 lines, pure Python):**
```python
# bindings/python/synddb.py
import ctypes
import sqlite3

lib = ctypes.CDLL('libsynddb.so')
lib.synddb_attach.argtypes = [ctypes.c_void_p, ctypes.c_char_p]
lib.synddb_attach.restype = ctypes.c_void_p

def attach(conn, url):
    handle = lib.synddb_attach(id(conn), url.encode())
    return handle
```

**That's it.** No setuptools, no compiler, no build scripts.

**Node.js binding (50 lines, pure JS):**
```javascript
// bindings/nodejs/synddb.js
const ffi = require('ffi-napi');

const lib = ffi.Library('libsynddb', {
  'synddb_attach': ['pointer', ['pointer', 'string']],
});

function attach(db, options) {
  return lib.synddb_attach(db.handle, options.sequencerUrl);
}
```

**That's it.** No node-gyp, no native modules, no compilation.

**Go binding (50 lines, pure Go):**
```go
// bindings/go/synddb.go
package synddb

// #cgo LDFLAGS: -lsynddb
// #include <synddb.h>
import "C"

func Attach(db *sql.DB, url string) *Handle {
    return C.synddb_attach(db.conn, C.CString(url))
}
```

**That's it.** cgo handles linking automatically.

### 3. This is How SQLite Works

SQLite is the most widely deployed database. It uses this exact model:

```
libsqlite3.so (C library)
├── Python: import sqlite3 (uses ctypes/builtin)
├── Node.js: better-sqlite3 (uses ffi-napi)
├── Go: github.com/mattn/go-sqlite3 (uses cgo)
├── Rust: rusqlite (uses bindgen)
└── Every other language (uses FFI)
```

**We're just following SQLite's proven pattern.**

### 4. Platform Support

**Compilation:**
```bash
# Cross-compile for all platforms from CI
cargo build --target x86_64-unknown-linux-gnu --release
cargo build --target x86_64-apple-darwin --release
cargo build --target aarch64-unknown-linux-gnu --release
cargo build --target x86_64-pc-windows-gnu --release
```

**Distribution:**
```python
# Python wheel includes platform-specific .so
synddb-0.1.0-cp311-cp311-linux_x86_64.whl
synddb-0.1.0-cp311-cp311-macosx_11_0_arm64.whl
```

```json
// Node.js package includes platform-specific .so
{
  "os": ["linux", "darwin", "win32"],
  "cpu": ["x64", "arm64"],
  "files": ["lib/", "bindings/"]
}
```

## Comparison Table

| Aspect | Native per Lang | PyO3/Neon | C FFI |
|--------|----------------|-----------|-------|
| **Core code** | Duplicate | Single | Single |
| **Binding complexity** | N/A | High | Low |
| **Build requirements** | Many | Rust + Target | Rust only |
| **Distribution** | Complex | Complex | Simple |
| **User install** | `pip install` | `pip install` (build) | `pip install` |
| **Maintenance** | High | Medium | Low |
| **Debugging** | Native tools | Mixed | Native tools |
| **Performance** | Good | Excellent | Excellent |

## Real-World Examples

**Using C FFI approach:**
- SQLite (C → all languages)
- libcurl (C → all languages)
- OpenSSL (C → all languages)
- ZeroMQ (C → all languages)

**Using native bindings:**
- TensorFlow (C++ with per-language bindings) - very complex
- gRPC (C++ with per-language bindings) - maintenance burden

## Recommendation

**Use Option C: Rust core with C FFI exports**

### Implementation Plan

1. **Core library (Rust):**
   - Implement in Rust as we've done
   - Add `#[no_mangle] pub extern "C"` functions for FFI
   - Compile to shared library: `libsynddb.so`

2. **C header:**
   - Simple header file: `synddb.h`
   - 4-5 function declarations
   - Can be auto-generated with `cbindgen`

3. **Language wrappers:**
   - Python: 50 lines using `ctypes` (pure Python)
   - Node.js: 50 lines using `ffi-napi` (pure JS)
   - Go: 50 lines using `cgo` (pure Go)
   - Rust: Use native library directly

4. **Distribution:**
   - Build binary for each platform in CI
   - Include binary in language packages
   - Users get pre-compiled library, zero build time

### Benefits

✅ **Simplest for users:** No compilation, just install and use
✅ **Simplest for us:** Single implementation, thin wrappers
✅ **Proven approach:** Same as SQLite, libcurl, etc.
✅ **Best performance:** Native code, no abstraction overhead
✅ **Easy debugging:** Native debuggers work in each language

### Trade-offs

⚠️ **Need to distribute binaries:** But this is standard practice
⚠️ **FFI boundary:** But it's trivial (4-5 functions)

## Conclusion

**The simplest cross-language approach is C FFI**, not native implementations per language or complex binding frameworks. It's what SQLite does, and it's proven to work at massive scale.

The key insight: **We're a library like SQLite, not an application.** Libraries use FFI.
