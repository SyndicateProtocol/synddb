//! C FFI exports for cross-language compatibility
//!
//! This module exposes a C ABI that can be called from any language
//! via standard FFI mechanisms (ctypes, cffi, ffi-napi, cgo, etc.)

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use rusqlite::Connection;
use crate::{SyndDB, Config};

/// Opaque handle to SyndDB instance
#[repr(C)]
pub struct SyndDBHandle {
    inner: Box<SyndDB>,
}

/// Attach to SQLite connection (C FFI)
///
/// # Arguments
/// * `conn_ptr` - Pointer to sqlite3 connection
/// * `sequencer_url` - Null-terminated C string with sequencer URL
///
/// # Returns
/// Opaque handle to SyndDB instance, or NULL on error
///
/// # Safety
/// Caller must ensure conn_ptr is valid for the lifetime of the handle
#[no_mangle]
pub unsafe extern "C" fn synddb_attach(
    conn_ptr: *mut std::ffi::c_void,
    sequencer_url: *const c_char,
) -> *mut SyndDBHandle {
    if conn_ptr.is_null() || sequencer_url.is_null() {
        return std::ptr::null_mut();
    }

    // Convert C string to Rust
    let url = match CStr::from_ptr(sequencer_url).to_str() {
        Ok(s) => s,
        Err(_) => return std::ptr::null_mut(),
    };

    // TODO: Convert conn_ptr to rusqlite Connection
    // This requires platform-specific code to wrap the raw pointer

    // For now, return placeholder
    std::ptr::null_mut()
}

/// Detach from SQLite connection and cleanup
///
/// # Safety
/// Handle must be valid pointer returned from synddb_attach
#[no_mangle]
pub unsafe extern "C" fn synddb_detach(handle: *mut SyndDBHandle) {
    if !handle.is_null() {
        drop(Box::from_raw(handle));
    }
}

/// Get last error message
///
/// # Returns
/// Null-terminated C string with error, or NULL if no error
#[no_mangle]
pub extern "C" fn synddb_last_error() -> *const c_char {
    // TODO: Implement thread-local error storage
    std::ptr::null()
}

/// Free error string returned by synddb_last_error
#[no_mangle]
pub unsafe extern "C" fn synddb_free_error(error: *mut c_char) {
    if !error.is_null() {
        drop(CString::from_raw(error));
    }
}
