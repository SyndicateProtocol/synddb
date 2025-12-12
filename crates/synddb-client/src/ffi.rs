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
/// * `flush_interval_ms` - Milliseconds between sender flushes (batching interval)
/// * `snapshot_interval` - Number of changesets between automatic snapshots (0 = disabled)
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
    flush_interval_ms: u64,
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
        flush_interval: Duration::from_millis(flush_interval_ms),
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

/// Manually publish all pending changesets
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
pub unsafe extern "C" fn synddb_publish(handle: *mut SyndDBHandle) -> SyndDBError {
    clear_last_error();

    if handle.is_null() {
        set_last_error("Null handle provided");
        return SyndDBError::InvalidPointer;
    }

    let synddb = &*(handle as *const SyndDB);

    match synddb.publish() {
        Ok(_) => SyndDBError::Success,
        Err(e) => {
            set_last_error(format!("Failed to publish: {}", e));
            SyndDBError::PublishError
        }
    }
}

/// Create a manual snapshot of the database
///
/// # Arguments
/// * `handle` - `SyndDB` handle from `synddb_attach()`
/// * `out_size` - Output pointer to receive snapshot size in bytes
///
/// # Returns
/// 0 on success, error code otherwise
///
/// # Safety
/// - `handle` must be a valid handle from `synddb_attach()`
/// - `out_size` must be a valid pointer
///
/// # Note
/// The snapshot data itself is sent directly to the sequencer.
/// This function only returns the size for informational purposes.
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
/// This will gracefully shutdown the client, publishing any pending changesets.
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
