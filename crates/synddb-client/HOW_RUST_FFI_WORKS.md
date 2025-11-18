# How Rust Compiles to C FFI

## TL;DR

Rust code stays Rust internally. At the boundary, we use `extern "C"` to expose functions with C-compatible calling conventions and types.

**It's like wrapping a Rust library in a C-shaped envelope.**

## The Layers

```
┌─────────────────────────────────────────────┐
│  Language Wrapper (Python/JS/Go)            │
│  Uses: ctypes, ffi-napi, cgo                │
│  Calls C functions via FFI                  │
└──────────────┬──────────────────────────────┘
               │ C ABI boundary
               │ (stable, universal interface)
               │
┌──────────────▼──────────────────────────────┐
│  FFI Layer (Rust with extern "C")           │
│  - Converts C types → Rust types            │
│  - Manages memory at boundary               │
│  - Catches panics, converts to errors       │
└──────────────┬──────────────────────────────┘
               │ Rust ABI
               │ (internal, can use any Rust features)
               │
┌──────────────▼──────────────────────────────┐
│  Core Implementation (Pure Rust)            │
│  - Async/await, generics, traits           │
│  - Safe Rust with borrowing, lifetimes     │
│  - Full Rust ecosystem                     │
└─────────────────────────────────────────────┘
```

## Example: Layer by Layer

### Layer 3: Core Implementation (Pure Rust)

```rust
// src/lib.rs - Normal Rust code
use rusqlite::Connection;

pub struct SyndDB {
    session: Session,
    sender: ChangesetSender,
}

impl SyndDB {
    pub fn attach(conn: &Connection, url: String) -> Result<Self> {
        // Async, borrowing, lifetimes - all normal Rust
        let session = Session::new(conn)?;
        let sender = ChangesetSender::new(url);

        Ok(Self { session, sender })
    }

    pub fn detach(self) -> Result<()> {
        self.sender.shutdown().await?;
        Ok(())
    }
}
```

**This is 100% normal Rust.** Uses all Rust features:
- References (`&Connection`)
- Ownership (`self`)
- `Result<T, E>` error handling
- Async/await
- Generic types

### Layer 2: FFI Boundary (Rust with C ABI)

```rust
// src/ffi.rs - C-compatible wrapper
use std::ffi::{CStr, CString};
use std::os::raw::c_char;

// Opaque handle (hides Rust implementation)
#[repr(C)]
pub struct SyndDBHandle {
    inner: Box<SyndDB>,  // Rust type wrapped in Box
}

/// C-compatible function
#[no_mangle]  // Don't mangle name (keep it as "synddb_attach")
pub extern "C" fn synddb_attach(
    conn_ptr: *mut std::ffi::c_void,  // Raw pointer (C-compatible)
    url: *const c_char,                // C string pointer
) -> *mut SyndDBHandle {               // Return pointer (C-compatible)

    // 1. Convert C types → Rust types
    if url.is_null() {
        return std::ptr::null_mut();
    }

    let url_str = unsafe {
        match CStr::from_ptr(url).to_str() {
            Ok(s) => s.to_string(),
            Err(_) => return std::ptr::null_mut(),
        }
    };

    // 2. Call Rust implementation
    // TODO: Convert conn_ptr to rusqlite::Connection
    // let conn = unsafe { ... };

    let synddb = match SyndDB::attach(&conn, url_str) {
        Ok(s) => s,
        Err(_) => return std::ptr::null_mut(),
    };

    // 3. Convert Rust type → C pointer
    let handle = Box::new(SyndDBHandle {
        inner: Box::new(synddb),
    });

    Box::into_raw(handle)  // Return raw pointer
}

#[no_mangle]
pub extern "C" fn synddb_detach(handle: *mut SyndDBHandle) {
    if !handle.is_null() {
        // Convert C pointer → Rust type
        let handle = unsafe { Box::from_raw(handle) };

        // Call Rust implementation
        let _ = handle.inner.detach();

        // Box automatically dropped here
    }
}
```

**Key elements:**

1. **`#[no_mangle]`**: Prevents Rust from renaming the function
   - Without it: `synddb_attach` → `_ZN10synddb_ffi13synddb_attach17h8b3d...`
   - With it: `synddb_attach` (C can find it)

2. **`extern "C"`**: Use C calling convention
   - Stack layout, register usage, etc. matches C
   - Any language can call it

3. **`#[repr(C)]`**: Memory layout matches C structs
   ```rust
   #[repr(C)]
   struct Point {
       x: i32,
       y: i32,
   }
   // Memory: [x: 4 bytes][y: 4 bytes] (same as C)
   ```

4. **C-compatible types only at boundary:**
   - `*mut c_void` (raw pointer)
   - `*const c_char` (C string)
   - `i32`, `u64` (primitive types)
   - NO: `&T`, `String`, `Result`, `Option`

### Layer 1: Language Wrapper (Python)

```python
# bindings/python/synddb.py
import ctypes

# Load the shared library
lib = ctypes.CDLL('libsynddb.so')

# Declare function signatures (must match C ABI)
lib.synddb_attach.argtypes = [
    ctypes.c_void_p,  # conn_ptr
    ctypes.c_char_p   # url
]
lib.synddb_attach.restype = ctypes.c_void_p  # Returns pointer

def attach(conn, url):
    # Python → C conversion
    conn_ptr = id(conn)  # Get pointer to Python object
    url_bytes = url.encode('utf-8')  # Python str → C string

    # Call C function
    handle = lib.synddb_attach(conn_ptr, url_bytes)

    if handle is None:
        raise RuntimeError("Failed to attach")

    return SyndDBHandle(handle)
```

## Compilation Process

### Step 1: Compile Rust to Shared Library

```bash
cargo build --release --lib
```

**What happens:**
1. Rust compiles all code to native machine code
2. Functions marked `extern "C"` get C calling convention
3. `#[no_mangle]` preserves function names
4. Linker creates shared library: `libsynddb.so`

### Step 2: Inspect the Binary

```bash
# See exported symbols
nm -D target/release/libsynddb.so | grep synddb

# Output:
00001a20 T synddb_attach      # "T" = exported function
00001b40 T synddb_detach
00001c60 T synddb_last_error
```

**These symbols look exactly like C functions to other languages.**

### Step 3: Generate C Header (Optional)

Use `cbindgen` to auto-generate C header from Rust:

```bash
cbindgen --lang c --output synddb.h
```

**Produces:**
```c
// synddb.h
typedef struct SyndDBHandle SyndDBHandle;

SyndDBHandle* synddb_attach(void* conn, const char* url);
void synddb_detach(SyndDBHandle* handle);
const char* synddb_last_error(void);
```

## What's Actually in the Binary?

Let's look at what the compiled library contains:

```bash
# Disassemble the function
objdump -d target/release/libsynddb.so | grep -A 20 synddb_attach
```

**You'll see native x86_64/ARM assembly code:**
```asm
0000000000001a20 <synddb_attach>:
    1a20: push   rbp
    1a21: mov    rbp, rsp
    1a24: sub    rsp, 0x30
    1a28: mov    QWORD PTR [rbp-0x8], rdi   # First arg (conn_ptr)
    1a2c: mov    QWORD PTR [rbp-0x10], rsi  # Second arg (url)
    ...
    # Rust code compiled to assembly
    # But uses C calling convention for this function
```

**It's machine code, not bytecode.** Same as C would produce.

## Memory Management at the Boundary

This is the tricky part. Here's how it works:

### Rust → C (Returning Pointer)

```rust
#[no_mangle]
pub extern "C" fn synddb_attach(...) -> *mut SyndDBHandle {
    let synddb = SyndDB::attach(...)?;

    // Allocate on heap, return pointer
    let handle = Box::new(SyndDBHandle {
        inner: Box::new(synddb),
    });

    // Transfer ownership to C
    Box::into_raw(handle)  // Prevents Rust from dropping it
}
```

**Memory layout:**
```
Heap:
┌─────────────────────┐
│  SyndDBHandle       │ ← Pointer returned to C
│  ┌───────────────┐  │
│  │ SyndDB        │  │
│  │ - session     │  │
│  │ - sender      │  │
│  └───────────────┘  │
└─────────────────────┘
```

### C → Rust (Taking Back Pointer)

```rust
#[no_mangle]
pub extern "C" fn synddb_detach(handle: *mut SyndDBHandle) {
    if !handle.is_null() {
        // Take ownership back from C
        let handle = unsafe { Box::from_raw(handle) };

        // Call Rust methods
        let _ = handle.inner.detach();

        // Box dropped here, memory freed
    }
}
```

## Type Conversions

### Primitive Types (Easy)

| C Type | Rust Type | Notes |
|--------|-----------|-------|
| `int` | `i32` | Same size, same representation |
| `unsigned long` | `u64` | Same size on 64-bit |
| `float` | `f32` | IEEE 754 |
| `double` | `f64` | IEEE 754 |

### Pointers (Medium)

| C Type | Rust Type | Notes |
|--------|-----------|-------|
| `void*` | `*mut c_void` | Raw pointer |
| `const char*` | `*const c_char` | C string |
| `int*` | `*mut i32` | Mutable pointer |

### Complex Types (Hard - Need Conversion)

| Rust Type | Cannot Cross FFI | Must Convert To |
|-----------|------------------|-----------------|
| `String` | ❌ Not C-compatible | `*const c_char` (CString) |
| `&str` | ❌ Fat pointer | `*const c_char` |
| `Result<T, E>` | ❌ Rust-specific | `*mut T` (NULL on error) |
| `Option<T>` | ❌ Rust-specific | `*mut T` (NULL for None) |
| `Vec<T>` | ❌ Fat pointer | `*mut T + length` |

## Example: String Conversion

```rust
// Rust → C string
fn rust_to_c_string(s: String) -> *const c_char {
    let c_string = CString::new(s).unwrap();
    c_string.into_raw()  // Transfer ownership to C
}

// C → Rust string
unsafe fn c_to_rust_string(ptr: *const c_char) -> String {
    CStr::from_ptr(ptr).to_string_lossy().into_owned()
}

// Free C string from Rust side
unsafe fn free_c_string(ptr: *mut c_char) {
    if !ptr.is_null() {
        drop(CString::from_raw(ptr));
    }
}
```

## Error Handling Across FFI

Rust's `Result` can't cross the boundary. We use C conventions:

```rust
// Rust side
thread_local! {
    static LAST_ERROR: RefCell<Option<String>> = RefCell::new(None);
}

fn set_last_error(err: String) {
    LAST_ERROR.with(|e| *e.borrow_mut() = Some(err));
}

#[no_mangle]
pub extern "C" fn synddb_attach(...) -> *mut SyndDBHandle {
    match internal_attach(...) {
        Ok(handle) => Box::into_raw(Box::new(handle)),
        Err(e) => {
            set_last_error(e.to_string());
            std::ptr::null_mut()  // Return NULL on error
        }
    }
}

#[no_mangle]
pub extern "C" fn synddb_last_error() -> *const c_char {
    LAST_ERROR.with(|e| {
        e.borrow()
            .as_ref()
            .map(|s| CString::new(s.clone()).unwrap().into_raw())
            .unwrap_or(std::ptr::null())
    })
}
```

**C-style error checking:**
```c
SyndDBHandle* handle = synddb_attach(conn, url);
if (handle == NULL) {
    const char* error = synddb_last_error();
    printf("Error: %s\n", error);
}
```

## Panic Safety

Rust panics can't unwind into C. We catch them:

```rust
use std::panic::catch_unwind;

#[no_mangle]
pub extern "C" fn synddb_attach(...) -> *mut SyndDBHandle {
    // Catch any panics
    let result = catch_unwind(|| {
        // Rust code that might panic
        SyndDB::attach(...)
    });

    match result {
        Ok(Ok(synddb)) => Box::into_raw(Box::new(synddb)),
        Ok(Err(e)) => {
            set_last_error(e.to_string());
            std::ptr::null_mut()
        }
        Err(_) => {
            set_last_error("Panic occurred".to_string());
            std::ptr::null_mut()
        }
    }
}
```

## Build Configuration

```toml
# Cargo.toml
[lib]
# Compile as both:
# - rlib: For Rust crates that depend on us
# - cdylib: For C FFI (shared library)
crate-type = ["rlib", "cdylib"]
```

```bash
# Build produces both:
cargo build --release

# Creates:
# - librlib/libsynddb_client.rlib  (for Rust)
# - libsynddb.so                    (for C FFI)
```

## Complete Example

**Rust (src/ffi.rs):**
```rust
#[repr(C)]
pub struct Handle { inner: Box<SyndDB> }

#[no_mangle]
pub extern "C" fn synddb_attach(
    conn: *mut c_void,
    url: *const c_char
) -> *mut Handle {
    let url = unsafe { CStr::from_ptr(url).to_str().unwrap() };
    let synddb = SyndDB::attach(...).unwrap();
    Box::into_raw(Box::new(Handle { inner: Box::new(synddb) }))
}

#[no_mangle]
pub extern "C" fn synddb_detach(handle: *mut Handle) {
    if !handle.is_null() {
        unsafe { drop(Box::from_raw(handle)) };
    }
}
```

**C Header (synddb.h):**
```c
typedef struct Handle Handle;

Handle* synddb_attach(void* conn, const char* url);
void synddb_detach(Handle* handle);
```

**Compiled Binary:**
```bash
$ nm -D libsynddb.so | grep synddb
00001a20 T synddb_attach    # Exported C symbol
00001b40 T synddb_detach
```

**Python Usage:**
```python
import ctypes

lib = ctypes.CDLL('libsynddb.so')
lib.synddb_attach.argtypes = [ctypes.c_void_p, ctypes.c_char_p]
lib.synddb_attach.restype = ctypes.c_void_p

handle = lib.synddb_attach(conn_ptr, b"http://...")
lib.synddb_detach(handle)
```

## Summary

**Question:** How does Rust compile down to C FFI?

**Answer:**
1. **Internally:** 100% Rust code with all Rust features
2. **At boundary:** `extern "C"` functions use C calling convention and C-compatible types
3. **Compilation:** Produces native machine code in a shared library (`.so`)
4. **Symbols:** Exported functions look like C functions to other languages
5. **No runtime:** No garbage collector, no runtime overhead
6. **Direct calls:** Other languages call Rust just like they'd call C

**It's Rust under the hood, C at the door.**
