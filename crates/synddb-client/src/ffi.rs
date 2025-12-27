//! C FFI interface for cross-language bindings
//!
//! This module exports a minimal C API that can be called from any language
//! with C FFI support (Python ctypes, Node.js ffi-napi, Go cgo, etc.)

use crate::{Config, SyndDB};
use rusqlite::Connection;
use std::{
    cell::RefCell,
    ffi::{CStr, CString},
    os::raw::c_char,
    time::Duration,
};
use tracing::{info, warn};

thread_local! {
    static LAST_ERROR: RefCell<Option<CString>> = const { RefCell::new(None) };
}

fn set_last_error(err: impl std::fmt::Display) {
    let error_msg = CString::new(format!("{}", err))
        .unwrap_or_else(|_| CString::new("Failed to format error message").unwrap());
    LAST_ERROR.with(|e| {
        *e.borrow_mut() = Some(error_msg);
    });
}

fn clear_last_error() {
    LAST_ERROR.with(|e| {
        *e.borrow_mut() = None;
    });
}

/// Opaque handle to `SyndDB` client for FFI
#[repr(C)]
#[derive(Debug)]
pub struct SyndDBHandle {
    _private: [u8; 0],
}

/// FFI-safe error code
#[repr(C)]
#[derive(Debug)]
pub enum SyndDBError {
    Success = 0,
    InvalidPointer = 1,
    InvalidUtf8 = 2,
    DatabaseError = 3,
    AttachError = 4,
    PublishError = 5,
    SnapshotError = 6,
    InvalidUrl = 7,
}

/// Attach `SyndDB` to a `SQLite` database file
///
/// # Arguments
/// * `db_path` - Path to `SQLite` database file (UTF-8 C string)
/// * `sequencer_url` - URL of sequencer TEE (UTF-8 C string)
/// * `out_handle` - Output pointer to receive `SyndDB` handle
///
/// # Returns
/// 0 on success, error code otherwise
///
/// # Safety
/// - `db_path` and `sequencer_url` must be valid null-terminated UTF-8 strings
/// - `out_handle` must be a valid pointer
/// - Caller must call `synddb_detach()` to free resources
///
/// # Example (C)
/// ```c
/// SyndDBHandle* handle;
/// int result = synddb_attach("app.db", "http://localhost:8433", &handle);
/// if (result != 0) {
///     fprintf(stderr, "Failed to attach SyndDB\n");
/// }
/// ```
#[no_mangle]
pub unsafe extern "C" fn synddb_attach(
    db_path: *const c_char,
    sequencer_url: *const c_char,
    out_handle: *mut *mut SyndDBHandle,
) -> SyndDBError {
    clear_last_error();

    if db_path.is_null() || sequencer_url.is_null() || out_handle.is_null() {
        set_last_error("Null pointer provided");
        return SyndDBError::InvalidPointer;
    }

    // Convert C strings to Rust strings
    let db_path_str = match CStr::from_ptr(db_path).to_str() {
        Ok(s) => s,
        Err(e) => {
            set_last_error(format!("Invalid UTF-8 in db_path: {}", e));
            return SyndDBError::InvalidUtf8;
        }
    };

    let sequencer_url_str = match CStr::from_ptr(sequencer_url).to_str() {
        Ok(s) => s,
        Err(e) => {
            set_last_error(format!("Invalid UTF-8 in sequencer_url: {}", e));
            return SyndDBError::InvalidUtf8;
        }
    };

    // Open database connection
    // SAFETY: We leak the connection to get 'static lifetime required by SyndDB
    // This is the expected pattern for long-lived database connections
    let conn = match Connection::open(db_path_str) {
        Ok(c) => Box::leak(Box::new(c)),
        Err(e) => {
            set_last_error(format!("Failed to open database: {}", e));
            return SyndDBError::DatabaseError;
        }
    };

    // Attach SyndDB with default config
    let synddb = match SyndDB::attach(conn, sequencer_url_str) {
        Ok(s) => s,
        Err(e) => {
            set_last_error(format!("Failed to attach SyndDB: {}", e));
            return SyndDBError::AttachError;
        }
    };

    // Box the SyndDB and return as opaque handle
    let boxed = Box::new(synddb);
    *out_handle = Box::into_raw(boxed) as *mut SyndDBHandle;

    SyndDBError::Success
}

/// Attach `SyndDB` with custom configuration
///
/// # Arguments
/// * `db_path` - Path to `SQLite` database file
/// * `sequencer_url` - URL of sequencer TEE
/// * `push_interval_ms` - Milliseconds between automatic pushes (must be > 0)
/// * `snapshot_interval` - Number of changesets between automatic snapshots (must be > 0)
/// * `out_handle` - Output pointer to receive `SyndDB` handle
///
/// # Returns
/// 0 on success, error code otherwise
///
/// # Safety
/// - `db_path` and `sequencer_url` must be valid null-terminated UTF-8 strings
/// - `out_handle` must be a valid pointer
/// - Caller must call `synddb_detach()` to free resources
#[no_mangle]
pub unsafe extern "C" fn synddb_attach_with_config(
    db_path: *const c_char,
    sequencer_url: *const c_char,
    push_interval_ms: u64,
    snapshot_interval: u64,
    out_handle: *mut *mut SyndDBHandle,
) -> SyndDBError {
    clear_last_error();

    if db_path.is_null() || sequencer_url.is_null() || out_handle.is_null() {
        set_last_error("Null pointer provided");
        return SyndDBError::InvalidPointer;
    }

    let db_path_str = match CStr::from_ptr(db_path).to_str() {
        Ok(s) => s,
        Err(e) => {
            set_last_error(format!("Invalid UTF-8 in db_path: {}", e));
            return SyndDBError::InvalidUtf8;
        }
    };

    let sequencer_url_str = match CStr::from_ptr(sequencer_url).to_str() {
        Ok(s) => s,
        Err(e) => {
            set_last_error(format!("Invalid UTF-8 in sequencer_url: {}", e));
            return SyndDBError::InvalidUtf8;
        }
    };

    let conn = match Connection::open(db_path_str) {
        Ok(c) => Box::leak(Box::new(c)),
        Err(e) => {
            set_last_error(format!("Failed to open database: {}", e));
            return SyndDBError::DatabaseError;
        }
    };

    let sequencer_url = match sequencer_url_str.parse() {
        Ok(url) => url,
        Err(e) => {
            set_last_error(format!("Invalid sequencer URL: {}", e));
            return SyndDBError::InvalidUrl;
        }
    };

    let config = Config {
        sequencer_url,
        push_interval: Duration::from_millis(push_interval_ms),
        snapshot_interval,
        ..Default::default()
    };

    let synddb = match SyndDB::attach_with_config(conn, config) {
        Ok(s) => s,
        Err(e) => {
            set_last_error(format!("Failed to attach SyndDB: {}", e));
            return SyndDBError::AttachError;
        }
    };

    let boxed = Box::new(synddb);
    *out_handle = Box::into_raw(boxed) as *mut SyndDBHandle;

    SyndDBError::Success
}

/// Send all pending changesets to the sequencer immediately
///
/// Changesets are automatically sent on a timer. Use this to force
/// immediate send for low-latency or high-value changes.
///
/// # Arguments
/// * `handle` - `SyndDB` handle from `synddb_attach()`
///
/// # Returns
/// 0 on success, error code otherwise
///
/// # Safety
/// - `handle` must be a valid handle from `synddb_attach()`
#[no_mangle]
pub unsafe extern "C" fn synddb_push(handle: *mut SyndDBHandle) -> SyndDBError {
    clear_last_error();

    if handle.is_null() {
        set_last_error("Null handle provided");
        return SyndDBError::InvalidPointer;
    }

    let synddb = &*(handle as *const SyndDB);

    match synddb.push() {
        Ok(_) => SyndDBError::Success,
        Err(e) => {
            set_last_error(format!("Failed to send changeset: {}", e));
            SyndDBError::PublishError
        }
    }
}

/// Create and publish a snapshot to the sequencer
///
/// This creates a complete database snapshot and sends it to the sequencer.
/// The snapshot includes the full database state (schema + data) and is used
/// for replica synchronization and disaster recovery.
///
/// # Behavior
///
/// This function is consistent with `synddb_push()` for changesets:
/// - `synddb_push()` - sends pending changesets to sequencer
/// - `synddb_snapshot()` - creates database snapshot and sends to sequencer
///
/// Both operations send data to the sequencer immediately (synchronous).
///
/// # When to Use
///
/// - After schema changes (`CREATE TABLE`, `ALTER TABLE`, etc.)
/// - To create periodic recovery checkpoints
/// - Before major migrations
///
/// Note: Schema changes (DDL) are NOT captured in changesets. You must call
/// this function after DDL to ensure validators can reconstruct the schema.
///
/// # Arguments
/// * `handle` - `SyndDB` handle from `synddb_attach()`
/// * `out_size` - Output pointer to receive snapshot size in bytes (optional, can be NULL)
///
/// # Returns
/// 0 on success, error code otherwise
///
/// # Safety
/// - `handle` must be a valid handle from `synddb_attach()`
/// - `out_size` can be NULL if size is not needed
///
/// # Example (Python)
/// ```python
/// # After creating schema
/// synddb.execute_batch("CREATE TABLE users (id INTEGER PRIMARY KEY)")
/// size = synddb.snapshot()  # Creates AND publishes to sequencer
/// print(f"Published {size} byte snapshot")
/// ```
#[no_mangle]
pub unsafe extern "C" fn synddb_snapshot(
    handle: *mut SyndDBHandle,
    out_size: *mut usize,
) -> SyndDBError {
    clear_last_error();

    if handle.is_null() {
        set_last_error("Null handle provided");
        return SyndDBError::InvalidPointer;
    }

    let synddb = &*(handle as *const SyndDB);

    // Create AND publish snapshot to the sequencer (synchronous)
    match synddb.snapshot() {
        Ok(snapshot) => {
            if !out_size.is_null() {
                *out_size = snapshot.data.len();
            }
            SyndDBError::Success
        }
        Err(e) => {
            set_last_error(format!("Failed to create snapshot: {}", e));
            SyndDBError::SnapshotError
        }
    }
}

/// Detach `SyndDB` and free resources
///
/// This will gracefully shutdown the client, sending any pending changesets.
///
/// # Arguments
/// * `handle` - `SyndDB` handle from `synddb_attach()`
///
/// # Safety
/// - `handle` must be a valid handle from `synddb_attach()`
/// - `handle` must not be used after this call
#[no_mangle]
pub unsafe extern "C" fn synddb_detach(handle: *mut SyndDBHandle) {
    if handle.is_null() {
        return;
    }

    // Reconstruct Box and let it drop (calls shutdown)
    let synddb = Box::from_raw(handle as *mut SyndDB);
    drop(synddb);
}

/// Get error message for the last error
///
/// # Returns
/// Pointer to null-terminated UTF-8 string describing the error, or null if no error.
///
/// # Safety
/// The returned pointer is valid until the next FFI call on the same thread.
/// The string must not be freed by the caller.
///
/// # Example (C)
/// ```c
/// if (synddb_attach(...) != 0) {
///     const char* error = synddb_last_error();
///     if (error) {
///         fprintf(stderr, "Error: %s\n", error);
///     }
/// }
/// ```
#[no_mangle]
pub extern "C" fn synddb_last_error() -> *const c_char {
    LAST_ERROR.with(|e| e.borrow().as_ref().map_or(std::ptr::null(), |s| s.as_ptr()))
}

/// Get library version string
///
/// # Returns
/// Pointer to null-terminated UTF-8 string with version (e.g., "0.1.0")
#[no_mangle]
pub extern "C" fn synddb_version() -> *const c_char {
    static VERSION: &[u8] = concat!(env!("CARGO_PKG_VERSION"), "\0").as_bytes();
    VERSION.as_ptr() as *const c_char
}

/// Execute a single SQL statement on the monitored connection
///
/// This is the correct way to write data when using `SyndDB` from FFI.
/// Changes made through this function are captured and published to the sequencer.
///
/// # Arguments
/// * `handle` - `SyndDB` handle from `synddb_attach()`
/// * `sql` - SQL statement to execute (UTF-8 C string)
///
/// # Returns
/// Number of rows affected on success, or -1 on error.
/// Call `synddb_last_error()` to get the error message.
///
/// # Safety
/// - `handle` must be a valid handle from `synddb_attach()`
/// - `sql` must be a valid null-terminated UTF-8 string
///
/// # Example (Python)
/// ```python
/// rows = synddb_execute(handle, b"INSERT INTO prices VALUES (1, 'BTC', 50000)\0")
/// if rows < 0:
///     print(synddb_last_error())
/// ```
#[no_mangle]
pub unsafe extern "C" fn synddb_execute(handle: *mut SyndDBHandle, sql: *const c_char) -> i64 {
    clear_last_error();

    if handle.is_null() {
        set_last_error("Null handle provided");
        return -1;
    }

    if sql.is_null() {
        set_last_error("Null SQL string provided");
        return -1;
    }

    let sql_str = match CStr::from_ptr(sql).to_str() {
        Ok(s) => s,
        Err(e) => {
            set_last_error(format!("Invalid UTF-8 in SQL: {}", e));
            return -1;
        }
    };

    let synddb = &*(handle as *const SyndDB);
    let conn = synddb.connection();

    match conn.execute(sql_str, []) {
        Ok(rows) => rows as i64,
        Err(e) => {
            set_last_error(format!("SQL execution failed: {}", e));
            -1
        }
    }
}

/// Execute multiple SQL statements (batch) on the monitored connection
///
/// This is useful for executing schema creation or multiple statements at once.
/// Changes made through this function are captured and published to the sequencer.
///
/// **Automatic Snapshotting**: If DDL statements (CREATE, ALTER, DROP) are detected,
/// a snapshot is automatically published after execution. This ensures validators
/// can always reconstruct the schema without manual intervention.
///
/// # Arguments
/// * `handle` - `SyndDB` handle from `synddb_attach()`
/// * `sql` - SQL statements to execute (UTF-8 C string, semicolon-separated)
///
/// # Returns
/// 0 on success, error code otherwise.
///
/// # Safety
/// - `handle` must be a valid handle from `synddb_attach()`
/// - `sql` must be a valid null-terminated UTF-8 string
///
/// # Example (Python)
/// ```python
/// result = synddb_execute_batch(handle, b'''
///     CREATE TABLE IF NOT EXISTS prices (id INTEGER PRIMARY KEY, asset TEXT, price REAL);
///     CREATE INDEX IF NOT EXISTS idx_asset ON prices(asset);
/// \0''')
/// # Snapshot is automatically published - no manual snapshot() call needed!
/// ```
#[no_mangle]
pub unsafe extern "C" fn synddb_execute_batch(
    handle: *mut SyndDBHandle,
    sql: *const c_char,
) -> SyndDBError {
    clear_last_error();

    if handle.is_null() {
        set_last_error("Null handle provided");
        return SyndDBError::InvalidPointer;
    }

    if sql.is_null() {
        set_last_error("Null SQL string provided");
        return SyndDBError::InvalidPointer;
    }

    let sql_str = match CStr::from_ptr(sql).to_str() {
        Ok(s) => s,
        Err(e) => {
            set_last_error(format!("Invalid UTF-8 in SQL: {}", e));
            return SyndDBError::InvalidUtf8;
        }
    };

    let synddb = &*(handle as *const SyndDB);
    let conn = synddb.connection();

    // Execute the SQL batch
    if let Err(e) = conn.execute_batch(sql_str) {
        set_last_error(format!("SQL batch execution failed: {}", e));
        return SyndDBError::DatabaseError;
    }

    // Auto-snapshot after DDL (always enabled for FFI - simplest DX)
    if SyndDB::is_ddl(sql_str) {
        info!("DDL executed via FFI, creating automatic snapshot");
        if let Err(e) = synddb.snapshot() {
            warn!("Failed to auto-snapshot after DDL: {}. Continuing.", e);
            // Don't fail the execute - the DDL succeeded, snapshot is best-effort
        }
    }

    SyndDBError::Success
}

/// Begin a transaction on the monitored connection
///
/// Call this before executing multiple statements that should be atomic.
/// Must be followed by `synddb_commit()` or `synddb_rollback()`.
///
/// # Returns
/// 0 on success, error code otherwise.
///
/// # Safety
/// - `handle` must be a valid handle from `synddb_attach()`
#[no_mangle]
pub unsafe extern "C" fn synddb_begin(handle: *mut SyndDBHandle) -> SyndDBError {
    clear_last_error();

    if handle.is_null() {
        set_last_error("Null handle provided");
        return SyndDBError::InvalidPointer;
    }

    let synddb = &*(handle as *const SyndDB);
    let conn = synddb.connection();

    match conn.execute("BEGIN", []) {
        Ok(_) => SyndDBError::Success,
        Err(e) => {
            set_last_error(format!("Failed to begin transaction: {}", e));
            SyndDBError::DatabaseError
        }
    }
}

/// Commit the current transaction
///
/// # Returns
/// 0 on success, error code otherwise.
///
/// # Safety
/// - `handle` must be a valid handle from `synddb_attach()`
#[no_mangle]
pub unsafe extern "C" fn synddb_commit(handle: *mut SyndDBHandle) -> SyndDBError {
    clear_last_error();

    if handle.is_null() {
        set_last_error("Null handle provided");
        return SyndDBError::InvalidPointer;
    }

    let synddb = &*(handle as *const SyndDB);
    let conn = synddb.connection();

    match conn.execute("COMMIT", []) {
        Ok(_) => SyndDBError::Success,
        Err(e) => {
            set_last_error(format!("Failed to commit transaction: {}", e));
            SyndDBError::DatabaseError
        }
    }
}

/// Rollback the current transaction
///
/// # Returns
/// 0 on success, error code otherwise.
///
/// # Safety
/// - `handle` must be a valid handle from `synddb_attach()`
#[no_mangle]
pub unsafe extern "C" fn synddb_rollback(handle: *mut SyndDBHandle) -> SyndDBError {
    clear_last_error();

    if handle.is_null() {
        set_last_error("Null handle provided");
        return SyndDBError::InvalidPointer;
    }

    let synddb = &*(handle as *const SyndDB);
    let conn = synddb.connection();

    match conn.execute("ROLLBACK", []) {
        Ok(_) => SyndDBError::Success,
        Err(e) => {
            set_last_error(format!("Failed to rollback transaction: {}", e));
            SyndDBError::DatabaseError
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ptr;

    #[test]
    fn test_ffi_version() {
        unsafe {
            let version = synddb_version();
            let version_str = CStr::from_ptr(version).to_str().unwrap();
            assert!(!version_str.is_empty());
            assert!(version_str.starts_with("0."));
        }
    }

    #[test]
    fn test_ffi_null_handling() {
        unsafe {
            let mut handle: *mut SyndDBHandle = ptr::null_mut();
            let result = synddb_attach(ptr::null(), ptr::null(), &raw mut handle);
            assert!(matches!(result, SyndDBError::InvalidPointer));
        }
    }
}
