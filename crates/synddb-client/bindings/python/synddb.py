"""
SyndDB Python Client - Pure Python FFI wrapper

No compilation needed! Just drop this file into your project.

Usage:
    import sqlite3
    from synddb import attach

    conn = sqlite3.connect('app.db')
    attach(conn, 'https://sequencer:8433')

    # Use SQLite normally
    conn.execute("INSERT INTO trades ...")
"""

import ctypes
import ctypes.util
import sqlite3
import os
from typing import Optional

# Find libsynddb
_lib_name = ctypes.util.find_library('synddb')
if _lib_name is None:
    # Try common locations
    for path in ['/usr/local/lib/libsynddb.so', './libsynddb.so', '../target/release/libsynddb.so']:
        if os.path.exists(path):
            _lib_name = path
            break

if _lib_name is None:
    raise ImportError("libsynddb.so not found. Install with: cargo build --release")

# Load library
_lib = ctypes.CDLL(_lib_name)

# Define function signatures
_lib.synddb_attach.argtypes = [ctypes.c_void_p, ctypes.c_char_p]
_lib.synddb_attach.restype = ctypes.c_void_p

_lib.synddb_detach.argtypes = [ctypes.c_void_p]
_lib.synddb_detach.restype = None

_lib.synddb_last_error.argtypes = []
_lib.synddb_last_error.restype = ctypes.c_char_p


class SyndDBHandle:
    """Handle to SyndDB instance"""

    def __init__(self, handle: ctypes.c_void_p):
        self._handle = handle

    def __del__(self):
        if self._handle:
            _lib.synddb_detach(self._handle)
            self._handle = None

    def detach(self):
        """Explicitly detach and flush"""
        if self._handle:
            _lib.synddb_detach(self._handle)
            self._handle = None


def attach(conn: sqlite3.Connection, sequencer_url: str) -> Optional[SyndDBHandle]:
    """
    Attach SyndDB to SQLite connection

    Args:
        conn: sqlite3.Connection instance
        sequencer_url: URL of sequencer (e.g. 'https://sequencer:8433')

    Returns:
        SyndDBHandle instance, or None on error

    Example:
        >>> import sqlite3
        >>> from synddb import attach
        >>>
        >>> conn = sqlite3.connect('app.db')
        >>> synddb = attach(conn, 'https://sequencer:8433')
        >>>
        >>> # Use SQLite normally
        >>> conn.execute("INSERT INTO trades VALUES (?, ?)", (1, 100))
    """
    # Get raw sqlite3* pointer from Python connection
    # This is implementation-dependent and may need adjustment
    conn_ptr = id(conn)  # TODO: Get actual sqlite3* pointer

    url_bytes = sequencer_url.encode('utf-8')
    handle = _lib.synddb_attach(conn_ptr, url_bytes)

    if handle is None:
        error = _lib.synddb_last_error()
        if error:
            raise RuntimeError(f"Failed to attach SyndDB: {error.decode('utf-8')}")
        else:
            raise RuntimeError("Failed to attach SyndDB: Unknown error")

    return SyndDBHandle(handle)


def last_error() -> Optional[str]:
    """Get last error message"""
    error = _lib.synddb_last_error()
    return error.decode('utf-8') if error else None
