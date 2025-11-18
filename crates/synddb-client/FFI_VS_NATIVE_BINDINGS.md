# FFI vs Native Bindings: Which Approach for SyndDB?

## TL;DR

**We're using C FFI (Approach 2), not PyO3/Neon.**

This is the SQLite model: compile Rust to a shared library with C ABI, then use thin wrappers in each language.

## The Two Approaches Explained

### Approach 1: Native Bindings (PyO3, Neon, etc.) ❌ NOT USING

**What it is:**
- PyO3: Rust macros that compile to Python extension modules
- Neon: Rust framework for Node.js native addons
- Each language gets a custom-built native extension

**How it works:**
```rust
// src/python.rs (using PyO3)
#[pyfunction]
fn attach(conn: &PyAny, url: String) -> PyResult<()> {
    // Rust code with Python types
}

#[pymodule]
fn synddb(_py: Python, m: &PyModule) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(attach, m)?)?;
    Ok(())
}
```

**User experience:**
```bash
pip install synddb
# Downloads source
# Requires: Rust, cargo, C compiler
# Compiles for 2-5 minutes
# Produces: synddb.cpython-311-x86_64-linux-gnu.so
```

**Pros:**
- Type-safe Rust↔Language boundary
- Nice error handling
- Feels native in target language

**Cons:**
- **Requires Rust on user's machine**
- **2-5 minute compilation on install**
- **Separate code for each language** (200+ lines per language)
- **30+ wheels to build** (Python versions × platforms × architectures)
- **Complex build system** (setuptools-rust, maturin, node-gyp, etc.)

### Approach 2: C FFI with Thin Wrappers ✅ USING THIS

**What it is:**
- Compile Rust once to shared library with C ABI
- Each language uses built-in FFI (ctypes, ffi-napi, cgo)
- Pure Python/JS/Go wrappers (~50 lines each)

**How it works:**
```rust
// src/ffi.rs (C exports)
#[no_mangle]
pub extern "C" fn synddb_attach(
    conn: *mut c_void,
    url: *const c_char
) -> *mut SyndDBHandle {
    // Rust implementation
}
```

```python
# bindings/python/synddb.py (pure Python)
import ctypes

lib = ctypes.CDLL('libsynddb.so')
lib.synddb_attach.argtypes = [ctypes.c_void_p, ctypes.c_char_p]

def attach(conn, url):
    return lib.synddb_attach(id(conn), url.encode())
```

**User experience:**
```bash
pip install synddb
# Downloads pre-compiled wheel
# Installs in <10 seconds
# No build tools needed
```

**Pros:**
- ✅ **No compilation for users** (pre-compiled .so)
- ✅ **Instant install** (pip/npm/go get in seconds)
- ✅ **Single binary** works for all languages
- ✅ **Thin wrappers** (~50 lines per language)
- ✅ **6 wheels total** (not 30+)
- ✅ **This is what SQLite does**

**Cons:**
- Manual memory management at boundary
- Raw pointers (less type-safe)
- C error conventions

## Side-by-Side Comparison

| Aspect | PyO3/Neon | C FFI |
|--------|-----------|-------|
| **User install time** | 2-5 minutes | <10 seconds |
| **Build tools required** | Rust, cargo, compiler | None |
| **Code per language** | 200+ lines | 50 lines |
| **Wheels to build** | 30+ (Python × OS × arch) | 6 (OS × arch) |
| **Binary distribution** | Complex | Simple |
| **Type safety** | High | Medium |
| **Maintenance** | Medium | Low |
| **Used by** | Some modern libs | SQLite, curl, OpenSSL |

## Real-World Examples

### Using C FFI (Approach 2):
- ✅ **SQLite** - 1B+ deployments
- ✅ **libcurl** - HTTP everywhere
- ✅ **OpenSSL** - Crypto everywhere
- ✅ **libsodium** - Modern crypto
- ✅ **ZeroMQ** - Messaging

### Using Native Bindings (Approach 1):
- ⚠️ **cryptography** (Python) - switched to PyO3, now needs 30+ wheels
- ⚠️ **TensorFlow** - complex per-language bindings, maintenance burden
- ⚠️ **gRPC** - separate implementation per language, lots of churn

## Why We Chose C FFI

### 1. User Experience Matters

**PyO3/Neon:**
```bash
$ pip install synddb
Collecting synddb
  Downloading synddb-0.1.0.tar.gz
  Installing build dependencies ... done
  Building wheel for synddb (pyproject.toml) ...
    [Compiling Rust code]
    cargo build --release
    [2-5 minutes of compilation]
  Successfully built synddb-0.1.0-cp311-cp311-linux_x86_64.whl
Successfully installed synddb-0.1.0

Time: 2-5 minutes
Requires: Rust, cargo, C compiler
```

**C FFI:**
```bash
$ pip install synddb
Collecting synddb
  Downloading synddb-0.1.0-py3-none-linux_x86_64.whl (2.1 MB)
Installing collected packages: synddb
Successfully installed synddb-0.1.0

Time: <10 seconds
Requires: Nothing (pre-compiled binary)
```

### 2. Distribution Simplicity

**PyO3 wheel matrix:**
```
synddb-0.1.0-cp38-cp38-linux_x86_64.whl
synddb-0.1.0-cp38-cp38-macosx_11_0_arm64.whl
synddb-0.1.0-cp38-cp38-win_amd64.whl
synddb-0.1.0-cp39-cp39-linux_x86_64.whl
synddb-0.1.0-cp39-cp39-macosx_11_0_arm64.whl
synddb-0.1.0-cp39-cp39-win_amd64.whl
... (30+ wheels)
```

**C FFI wheel matrix:**
```
synddb-0.1.0-py3-none-linux_x86_64.whl
synddb-0.1.0-py3-none-macosx_11_0_arm64.whl
synddb-0.1.0-py3-none-win_amd64.whl
... (6 wheels)
```

### 3. This Is What SQLite Does

SQLite is the most deployed database engine in the world. It uses C FFI:

```
Build libsqlite3.so once
└─ Use from:
   ├─ Python (ctypes)
   ├─ Node.js (ffi-napi)
   ├─ Go (cgo)
   ├─ Rust (bindgen)
   ├─ Java (JNI)
   └─ 100+ other languages
```

**If it's good enough for SQLite, it's good enough for us.**

### 4. Our Needs Are Simple

PyO3/Neon make sense when:
- Complex API with many types
- Tight integration with language runtime
- Performance-critical callbacks

Our API is trivial:
```c
synddb_handle* synddb_attach(sqlite3* conn, const char* url);
void synddb_detach(synddb_handle* handle);
const char* synddb_last_error(void);
```

**3 functions.** That's it. FFI overhead is negligible.

## Implementation Details

### What We Build

```bash
# Compile Rust to shared library
cargo build --release --lib

# Produces:
# - Linux: libsynddb.so
# - macOS: libsynddb.dylib
# - Windows: synddb.dll
```

### What Users Get

**Python:**
```python
# Pure Python wrapper (no compilation)
import ctypes
lib = ctypes.CDLL('libsynddb.so')
def attach(conn, url):
    return lib.synddb_attach(conn, url.encode())
```

**Node.js:**
```javascript
// Pure JS wrapper (no compilation)
const ffi = require('ffi-napi');
const lib = ffi.Library('libsynddb', {...});
function attach(db, options) {
    return lib.synddb_attach(db.handle, options.url);
}
```

**Go:**
```go
// Pure Go wrapper (cgo auto-links)
// #cgo LDFLAGS: -lsynddb
import "C"
func Attach(db *sql.DB, url string) {
    C.synddb_attach(db.conn, C.CString(url))
}
```

## Decision: C FFI

**We are using C FFI (Approach 2)** because:

1. ✅ Better user experience (instant install)
2. ✅ Simpler distribution (fewer wheels)
3. ✅ Less code to maintain (50 lines vs 200+ per language)
4. ✅ Proven at scale (SQLite model)
5. ✅ Our API is simple (3 functions)

The PyO3/Neon code in this repo was exploratory and **should be removed**.

## Clean Up Tasks

- [x] Remove PyO3 dependency from Cargo.toml
- [x] Remove Neon dependency from Cargo.toml
- [x] Remove src/python.rs (PyO3 bindings)
- [x] Keep src/ffi.rs (C exports)
- [x] Keep bindings/ directory (thin wrappers)
- [x] Update Cargo.toml with `crate-type = ["cdylib"]`

## References

- SQLite: https://www.sqlite.org/
- How SQLite uses FFI: https://github.com/rusqlite/rusqlite
- Rust FFI Guide: https://doc.rust-lang.org/nomicon/ffi.html
- Python ctypes: https://docs.python.org/3/library/ctypes.html
- Node ffi-napi: https://github.com/node-ffi-napi/node-ffi-napi
- Go cgo: https://go.dev/blog/cgo
