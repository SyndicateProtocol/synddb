# When to Use PyO3/Neon vs C FFI

## TL;DR

**Use PyO3/Neon when:**
- Complex API with many types/functions
- Need callbacks from target language → Rust
- Performance-critical language runtime integration
- You control the deployment (can require build tools)

**Use C FFI when:**
- Simple API (few functions)
- One-way calls (language → Rust)
- Want instant install for users
- Supporting many languages with one implementation

## The Trade-Off Matrix

| Factor | PyO3/Neon | C FFI |
|--------|-----------|-------|
| **API Complexity** | Best for 50+ functions | Best for <10 functions |
| **Callbacks** | Easy (language → Rust) | Hard (requires unsafe) |
| **Type Safety** | Excellent | Manual |
| **User Install** | 2-5 min compile | <10 sec |
| **Build Tools** | Required | None |
| **Multi-Language** | Per-language code | Single implementation |
| **Maintenance** | Medium | Low (simple API) |

## When PyO3/Neon Make Sense

### 1. Complex APIs with Many Functions

**Example: NumPy/Pandas (PyO3)**

NumPy has hundreds of functions:
```python
# Many functions, many types
array = np.array([1, 2, 3])
result = np.dot(array, array)
fourier = np.fft.fft(array)
filtered = np.where(array > 1, array, 0)
# ... hundreds more
```

**Why PyO3 is better here:**
- 500+ functions to expose
- Each needs proper type conversion
- Complex types: `ndarray`, `dtype`, `axis`
- Hand-writing FFI wrappers = maintenance nightmare

**With PyO3:**
```rust
#[pyclass]
struct Array {
    data: Vec<f64>,
}

#[pymethods]
impl Array {
    #[new]
    fn new(data: Vec<f64>) -> Self { ... }

    fn dot(&self, other: &Array) -> f64 { ... }

    fn fft(&self) -> Array { ... }

    fn where(&self, condition: bool, x: f64, y: f64) -> Array { ... }
}
```

**Type-safe, auto-generated bindings.** Worth the compilation cost.

**With C FFI:**
```c
// Would need to manually write:
double* array_new(double* data, size_t len);
double array_dot(double* a, double* b, size_t len);
double* array_fft(double* data, size_t len, size_t* out_len);
double* array_where(double* data, size_t len, bool cond, double x, double y);
// ... and 496 more functions
```

**Tedious and error-prone.**

### 2. Callbacks from Target Language → Rust

**Example: Event Handlers, User-Defined Functions**

```python
# User provides Python function
def my_filter(item):
    return item.value > 100

# Rust library calls it during processing
processor.filter(my_filter)
```

**Why PyO3 is better:**
```rust
#[pyfunction]
fn filter(data: Vec<PyObject>, predicate: PyObject) -> PyResult<Vec<PyObject>> {
    let mut result = Vec::new();
    for item in data {
        // Call Python function from Rust
        let keep: bool = predicate.call1(py, (item.clone(),))?.extract(py)?;
        if keep {
            result.push(item);
        }
    }
    Ok(result)
}
```

**PyO3 handles:**
- Acquiring/releasing Python GIL
- Converting Rust → Python objects
- Calling Python functions safely
- Exception handling

**With C FFI:**
```c
// Much harder - need to:
// 1. Get Python C API
// 2. Manually manage GIL
// 3. Convert types manually
// 4. Handle exceptions manually

typedef struct {
    PyObject* callable;
} Callback;

void* filter(void* data, size_t len, Callback* cb) {
    PyGILState_STATE gstate = PyGILState_Ensure();
    PyObject* args = PyTuple_New(1);
    // ... lots of manual C API calls
    PyGILState_Release(gstate);
}
```

**Very complex and error-prone.**

### 3. Performance-Critical Runtime Integration

**Example: Data Processing Library (Polars, Pydantic)**

When you need to:
- Zero-copy access to language runtime memory
- Share memory between Rust and language
- Avoid serialization overhead

**Polars (DataFrame library):**
```rust
#[pyclass]
struct DataFrame {
    inner: polars::DataFrame,
}

#[pymethods]
impl DataFrame {
    fn select(&self, columns: Vec<&str>) -> Self {
        // Direct access to Rust data structures
        // No serialization needed
        Self {
            inner: self.inner.select(columns).unwrap()
        }
    }
}
```

**Benefits:**
- Zero-copy data sharing
- Native Python types (no conversion)
- Tight integration with Python runtime

**With C FFI:**
```c
// Would need to serialize/deserialize
char* dataframe_select(void* df, char** columns, size_t len) {
    // Must serialize Rust DataFrame → JSON/bytes
    // Python deserializes → creates copy
    // Expensive for large datasets
}
```

### 4. You Control the Deployment

**Example: Internal Tools, ML Pipelines**

If users are:
- Data scientists with conda/build tools
- Internal teams with standard environments
- CI/CD systems that can compile

**Then compilation time is acceptable:**
```bash
# In a Dockerfile or CI
pip install your-library  # 2-5 min compile is fine
```

**Not a big deal if:**
- One-time setup per environment
- Build tools already available
- Installation is automated

## When C FFI Makes Sense

### 1. Simple API (Few Functions)

**Example: SyndDB Client**

Our API:
```c
synddb_handle* synddb_attach(sqlite3* conn, const char* url);
void synddb_detach(synddb_handle* handle);
const char* synddb_last_error(void);
```

**3 functions.** That's it.

**PyO3 overhead not worth it:**
- 200+ lines of binding code
- Compilation requirement
- For 3 simple functions

**C FFI is perfect:**
- 50 lines of wrapper per language
- No compilation
- Clear and simple

### 2. One-Way Calls (Language → Rust)

**Example: Compression Library, Crypto Library**

```python
# Simple one-way calls
compressed = zstd.compress(data)
hash = blake3.hash(data)
signature = ed25519.sign(message, key)
```

**No callbacks needed.** Just:
1. Language calls Rust
2. Rust does work
3. Rust returns result

**C FFI works great:**
```c
char* zstd_compress(char* data, size_t len, size_t* out_len);
char* blake3_hash(char* data, size_t len);
char* ed25519_sign(char* msg, size_t len, char* key);
```

Simple, fast, no runtime integration needed.

### 3. Supporting Many Languages

**Example: SQLite, libcurl, OpenSSL**

If you need to support:
- Python
- Node.js
- Go
- Rust
- Java
- Ruby
- PHP
- ... and more

**C FFI wins:**
- Write once, use everywhere
- Every language has C FFI
- Single binary

**PyO3/Neon loses:**
- PyO3 for Python
- Neon for Node.js
- jni for Java
- Different code for each = maintenance burden

### 4. Instant Install Critical

**Example: CLI tools, developer libraries**

Users expect:
```bash
pip install your-tool
your-tool --help  # Should work immediately
```

**Not:**
```bash
pip install your-tool
# ... 5 minutes of compilation ...
# ... potential build failures ...
# ... "error: Rust compiler not found" ...
your-tool --help
```

**C FFI = better UX** for general-purpose tools.

## Real-World Examples

### Projects Using PyO3 (Good Fit)

**1. cryptography (Python)**
- **Why:** 100+ crypto functions, complex types
- **Trade-off:** 30+ wheels to build, but worth it for API complexity

**2. Pydantic v2 (Python)**
- **Why:** High-performance validation, tight Python integration
- **Trade-off:** Compilation required, but core library for many projects

**3. polars (Python/Node.js)**
- **Why:** DataFrame library, zero-copy data sharing crucial
- **Trade-off:** Build time acceptable for data science workflows

**4. ruff (Python linter)**
- **Why:** Complex AST parsing, many rules
- **Trade-off:** Pre-built binaries for most platforms

### Projects Using C FFI (Good Fit)

**1. SQLite**
- **Why:** Simple API (50 functions), many languages
- **Result:** Used by billions of devices

**2. libsodium (crypto)**
- **Why:** Clean API, one-way calls, multi-language
- **Result:** Standard crypto library across languages

**3. libcurl (HTTP client)**
- **Why:** HTTP client API, used everywhere
- **Result:** De facto standard for HTTP

**4. zstd (compression)**
- **Why:** Simple compress/decompress API
- **Result:** Fast, universal compression

### Projects That Should Switch

**1. Some CLI tools built with PyO3**
- Simple command-line tools
- Don't need Python runtime integration
- Users struggle with compilation
- **Should use:** C FFI or pure Rust binary

## Decision Tree

```
Do you have 50+ functions to expose?
├─ Yes → Consider PyO3/Neon
└─ No  → Continue

Do you need callbacks from target language?
├─ Yes → Consider PyO3/Neon
└─ No  → Continue

Do you need zero-copy data sharing?
├─ Yes → Consider PyO3/Neon
└─ No  → Continue

Do you support multiple languages?
├─ Yes → Use C FFI
└─ No  → Continue

Is instant install critical?
├─ Yes → Use C FFI
└─ No  → PyO3/Neon is fine

Can you require build tools?
├─ Yes → PyO3/Neon is fine
└─ No  → Use C FFI
```

## For SyndDB Client

Let's apply this to our case:

**Questions:**

1. **Complex API?** No - 3 functions
2. **Callbacks needed?** No - one-way calls
3. **Zero-copy crucial?** No - small changesets
4. **Multiple languages?** Yes - Python, Node.js, Go
5. **Instant install important?** Yes - developer tool
6. **Can require build tools?** No - TEE environments

**Answer: 5/6 point to C FFI**

**Decision: C FFI is the right choice.**

## Summary

**Use PyO3/Neon when:**
```
✅ Complex API (50+ functions)
✅ Need callbacks (language → Rust → language)
✅ Performance-critical runtime integration
✅ You control deployment environment
✅ Single language focus
```

**Use C FFI when:**
```
✅ Simple API (<10 functions)
✅ One-way calls (language → Rust)
✅ Supporting many languages
✅ Instant install critical
✅ End-users without build tools
```

**For SyndDB:**
- ✅ Simple API (3 functions)
- ✅ One-way calls
- ✅ Multi-language (Python/JS/Go)
- ✅ Developer tool (fast install)

**C FFI is the clear winner.**

## Additional Resources

- **PyO3 Guide**: https://pyo3.rs/
- **Neon Guide**: https://neon-bindings.com/
- **Rust FFI Guide**: https://doc.rust-lang.org/nomicon/ffi.html
- **SQLite Architecture**: https://www.sqlite.org/arch.html (uses C FFI)
- **cryptography's switch to Rust**: https://blog.trailofbits.com/2021/02/23/good-practices-cryptography-rust/
