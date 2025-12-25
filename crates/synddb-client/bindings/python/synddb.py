"""
SyndDB Python Client - Pure Python FFI wrapper (no compilation needed!)

Usage:
    from synddb import SyndDB

    # Attach to database file
    synddb = SyndDB.attach('app.db', 'http://localhost:8433')

    # Use SQLite normally - changesets are automatically captured
    import sqlite3
    conn = sqlite3.connect('app.db')
    conn.execute("INSERT INTO trades VALUES (?, ?)", (1, 100))
    conn.commit()

    # IMPORTANT: Call publish() after commits to send changesets
    synddb.publish()

    # Create a snapshot (optional)
    synddb.snapshot()

    # Clean up (or let Python garbage collector handle it)
    synddb.detach()
"""

import ctypes
import ctypes.util
import os
import platform
from typing import Optional

# Find libsynddb shared library
def _find_library():
    """Locate the synddb shared library"""
    system = platform.system()

    if system == 'Darwin':  # macOS
        lib_name = 'libsynddb_client.dylib'
    elif system == 'Windows':
        lib_name = 'synddb_client.dll'
    else:  # Linux and others
        lib_name = 'libsynddb_client.so'

    # Try common locations
    search_paths = [
        # Current directory
        os.path.join('.', lib_name),
        # Relative to this file
        os.path.join(os.path.dirname(__file__), '..', '..', 'target', 'release', lib_name),
        os.path.join(os.path.dirname(__file__), '..', '..', 'target', 'debug', lib_name),
        # System paths
        os.path.join('/usr', 'local', 'lib', lib_name),
        os.path.join('/usr', 'lib', lib_name),
    ]

    for path in search_paths:
        if os.path.exists(path):
            return path

    # Try system library search
    lib_path = ctypes.util.find_library('synddb_client')
    if lib_path:
        return lib_path

    raise ImportError(
        f"libsynddb_client not found. Build with: cargo build --package synddb-client --features ffi --release"
    )

# Load library
_lib = ctypes.CDLL(_find_library())

# Error codes (must match Rust enum)
class SyndDBError:
    SUCCESS = 0
    INVALID_POINTER = 1
    INVALID_UTF8 = 2
    DATABASE_ERROR = 3
    ATTACH_ERROR = 4
    PUBLISH_ERROR = 5
    SNAPSHOT_ERROR = 6

# Opaque handle type
class _SyndDBHandle(ctypes.Structure):
    pass

# Define function signatures
_lib.synddb_attach.argtypes = [
    ctypes.c_char_p,  # db_path
    ctypes.c_char_p,  # sequencer_url
    ctypes.POINTER(ctypes.POINTER(_SyndDBHandle))  # out_handle
]
_lib.synddb_attach.restype = ctypes.c_int

_lib.synddb_attach_with_config.argtypes = [
    ctypes.c_char_p,  # db_path
    ctypes.c_char_p,  # sequencer_url
    ctypes.c_uint64,  # flush_interval_ms
    ctypes.c_uint64,  # snapshot_interval
    ctypes.POINTER(ctypes.POINTER(_SyndDBHandle))  # out_handle
]
_lib.synddb_attach_with_config.restype = ctypes.c_int

_lib.synddb_publish.argtypes = [ctypes.POINTER(_SyndDBHandle)]
_lib.synddb_publish.restype = ctypes.c_int

_lib.synddb_snapshot.argtypes = [
    ctypes.POINTER(_SyndDBHandle),
    ctypes.POINTER(ctypes.c_size_t)  # out_size (optional)
]
_lib.synddb_snapshot.restype = ctypes.c_int

_lib.synddb_detach.argtypes = [ctypes.POINTER(_SyndDBHandle)]
_lib.synddb_detach.restype = None

_lib.synddb_last_error.argtypes = []
_lib.synddb_last_error.restype = ctypes.c_char_p

_lib.synddb_version.argtypes = []
_lib.synddb_version.restype = ctypes.c_char_p

_lib.synddb_execute.argtypes = [ctypes.POINTER(_SyndDBHandle), ctypes.c_char_p]
_lib.synddb_execute.restype = ctypes.c_int64

_lib.synddb_execute_batch.argtypes = [ctypes.POINTER(_SyndDBHandle), ctypes.c_char_p]
_lib.synddb_execute_batch.restype = ctypes.c_int

_lib.synddb_begin.argtypes = [ctypes.POINTER(_SyndDBHandle)]
_lib.synddb_begin.restype = ctypes.c_int

_lib.synddb_commit.argtypes = [ctypes.POINTER(_SyndDBHandle)]
_lib.synddb_commit.restype = ctypes.c_int

_lib.synddb_rollback.argtypes = [ctypes.POINTER(_SyndDBHandle)]
_lib.synddb_rollback.restype = ctypes.c_int


class SyndDB:
    """SyndDB client handle - automatically captures and publishes SQLite changesets"""

    def __init__(self, handle: ctypes.POINTER(_SyndDBHandle)):
        self._handle = handle

    @classmethod
    def attach(cls, db_path: str, sequencer_url: str) -> 'SyndDB':
        """
        Attach SyndDB to a SQLite database file

        Args:
            db_path: Path to SQLite database file
            sequencer_url: URL of sequencer TEE (e.g., 'http://localhost:8433')

        Returns:
            SyndDB instance

        Raises:
            RuntimeError: If attachment fails

        Example:
            >>> synddb = SyndDB.attach('app.db', 'http://localhost:8433')
            >>>
            >>> # Now use SQLite normally
            >>> import sqlite3
            >>> conn = sqlite3.connect('app.db')
            >>> conn.execute("INSERT INTO users VALUES (?, ?)", (1, 'Alice'))
        """
        handle = ctypes.POINTER(_SyndDBHandle)()
        result = _lib.synddb_attach(
            db_path.encode('utf-8'),
            sequencer_url.encode('utf-8'),
            ctypes.byref(handle)
        )

        if result != SyndDBError.SUCCESS:
            error_msg = _lib.synddb_last_error()
            error_str = error_msg.decode('utf-8') if error_msg else "Unknown error"
            raise RuntimeError(f"Failed to attach SyndDB (error {result}): {error_str}")

        return cls(handle)

    @classmethod
    def attach_with_config(
        cls,
        db_path: str,
        sequencer_url: str,
        flush_interval_ms: int = 1000,
        snapshot_interval: int = 0
    ) -> 'SyndDB':
        """
        Attach SyndDB with custom configuration

        Args:
            db_path: Path to SQLite database file
            sequencer_url: URL of sequencer TEE
            flush_interval_ms: Milliseconds between automatic publishes (default: 1000)
            snapshot_interval: Changesets between snapshots (default: 0 = disabled)

        Returns:
            SyndDB instance

        Example:
            >>> synddb = SyndDB.attach_with_config(
            ...     'app.db',
            ...     'http://localhost:8433',
            ...     flush_interval_ms=500,  # Publish every 500ms
            ...     snapshot_interval=100      # Snapshot every 100 changesets
            ... )
        """
        handle = ctypes.POINTER(_SyndDBHandle)()
        result = _lib.synddb_attach_with_config(
            db_path.encode('utf-8'),
            sequencer_url.encode('utf-8'),
            flush_interval_ms,
            snapshot_interval,
            ctypes.byref(handle)
        )

        if result != SyndDBError.SUCCESS:
            error_msg = _lib.synddb_last_error()
            error_str = error_msg.decode('utf-8') if error_msg else "Unknown error"
            raise RuntimeError(f"Failed to attach SyndDB (error {result}): {error_str}")

        return cls(handle)

    def publish(self):
        """
        Publish all pending changesets to the sequencer

        Call this after committing transactions to send changesets to the sequencer.
        Also called automatically on detach for graceful shutdown.

        Raises:
            RuntimeError: If publish fails

        Example:
            >>> synddb.publish()
        """
        if not self._handle:
            raise RuntimeError("SyndDB handle already detached")

        result = _lib.synddb_publish(self._handle)

        if result != SyndDBError.SUCCESS:
            error_msg = _lib.synddb_last_error()
            error_str = error_msg.decode('utf-8') if error_msg else "Unknown error"
            raise RuntimeError(f"Failed to publish (error {result}): {error_str}")

    def snapshot(self) -> int:
        """
        Create a manual snapshot of the database

        Returns:
            Size of snapshot in bytes

        Raises:
            RuntimeError: If snapshot creation fails

        Example:
            >>> size = synddb.snapshot()
            >>> print(f"Snapshot created: {size} bytes")
        """
        if not self._handle:
            raise RuntimeError("SyndDB handle already detached")

        size = ctypes.c_size_t()
        result = _lib.synddb_snapshot(self._handle, ctypes.byref(size))

        if result != SyndDBError.SUCCESS:
            error_msg = _lib.synddb_last_error()
            error_str = error_msg.decode('utf-8') if error_msg else "Unknown error"
            raise RuntimeError(f"Failed to create snapshot (error {result}): {error_str}")

        return size.value

    def execute(self, sql: str) -> int:
        """
        Execute a SQL statement on the monitored connection.

        This is the correct way to write data when using SyndDB. Changes made
        through this method are captured and published to the sequencer.

        Args:
            sql: SQL statement to execute

        Returns:
            Number of rows affected

        Raises:
            RuntimeError: If execution fails

        Example:
            >>> rows = synddb.execute("INSERT INTO prices VALUES (1, 'BTC', 50000)")
            >>> print(f"Inserted {rows} row(s)")
        """
        if not self._handle:
            raise RuntimeError("SyndDB handle already detached")

        result = _lib.synddb_execute(self._handle, sql.encode('utf-8'))

        if result < 0:
            error_msg = _lib.synddb_last_error()
            error_str = error_msg.decode('utf-8') if error_msg else "Unknown error"
            raise RuntimeError(f"SQL execution failed: {error_str}")

        return result

    def execute_batch(self, sql: str) -> None:
        """
        Execute multiple SQL statements (batch) on the monitored connection.

        Useful for schema creation or multiple statements at once.

        Args:
            sql: SQL statements to execute (semicolon-separated)

        Raises:
            RuntimeError: If execution fails

        Example:
            >>> synddb.execute_batch('''
            ...     CREATE TABLE IF NOT EXISTS prices (id INTEGER PRIMARY KEY);
            ...     CREATE INDEX IF NOT EXISTS idx ON prices(id);
            ... ''')
        """
        if not self._handle:
            raise RuntimeError("SyndDB handle already detached")

        result = _lib.synddb_execute_batch(self._handle, sql.encode('utf-8'))

        if result != SyndDBError.SUCCESS:
            error_msg = _lib.synddb_last_error()
            error_str = error_msg.decode('utf-8') if error_msg else "Unknown error"
            raise RuntimeError(f"SQL batch execution failed: {error_str}")

    def begin(self) -> None:
        """
        Begin a transaction.

        Must be followed by commit() or rollback().

        Example:
            >>> synddb.begin()
            >>> synddb.execute("INSERT INTO prices VALUES (1, 'BTC', 50000)")
            >>> synddb.execute("INSERT INTO prices VALUES (2, 'ETH', 3000)")
            >>> synddb.commit()
        """
        if not self._handle:
            raise RuntimeError("SyndDB handle already detached")

        result = _lib.synddb_begin(self._handle)

        if result != SyndDBError.SUCCESS:
            error_msg = _lib.synddb_last_error()
            error_str = error_msg.decode('utf-8') if error_msg else "Unknown error"
            raise RuntimeError(f"Failed to begin transaction: {error_str}")

    def commit(self) -> None:
        """
        Commit the current transaction.

        Example:
            >>> synddb.begin()
            >>> synddb.execute("INSERT INTO prices VALUES (1, 'BTC', 50000)")
            >>> synddb.commit()
        """
        if not self._handle:
            raise RuntimeError("SyndDB handle already detached")

        result = _lib.synddb_commit(self._handle)

        if result != SyndDBError.SUCCESS:
            error_msg = _lib.synddb_last_error()
            error_str = error_msg.decode('utf-8') if error_msg else "Unknown error"
            raise RuntimeError(f"Failed to commit transaction: {error_str}")

    def rollback(self) -> None:
        """
        Rollback the current transaction.

        Example:
            >>> synddb.begin()
            >>> synddb.execute("INSERT INTO prices VALUES (1, 'BTC', 50000)")
            >>> synddb.rollback()  # Changes are discarded
        """
        if not self._handle:
            raise RuntimeError("SyndDB handle already detached")

        result = _lib.synddb_rollback(self._handle)

        if result != SyndDBError.SUCCESS:
            error_msg = _lib.synddb_last_error()
            error_str = error_msg.decode('utf-8') if error_msg else "Unknown error"
            raise RuntimeError(f"Failed to rollback transaction: {error_str}")

    def detach(self):
        """
        Detach SyndDB and free resources

        This gracefully shuts down the client, publishing any pending changesets.
        The instance cannot be used after this call.

        Example:
            >>> synddb.detach()
        """
        if self._handle:
            _lib.synddb_detach(self._handle)
            self._handle = None

    def __del__(self):
        """Automatically detach when garbage collected"""
        self.detach()

    def __enter__(self):
        """Context manager support"""
        return self

    def __exit__(self, exc_type, exc_val, exc_tb):
        """Context manager cleanup"""
        self.detach()


def version() -> str:
    """
    Get library version string

    Returns:
        Version string (e.g., "0.1.0")

    Example:
        >>> import synddb
        >>> print(synddb.version())
        0.1.0
    """
    version_bytes = _lib.synddb_version()
    return version_bytes.decode('utf-8')


def last_error() -> Optional[str]:
    """
    Get last error message

    Returns:
        Error message string, or None if no error
    """
    error_bytes = _lib.synddb_last_error()
    return error_bytes.decode('utf-8') if error_bytes else None


# Convenience function for quick setup
def attach(db_path: str, sequencer_url: str, **kwargs) -> SyndDB:
    """
    Convenience function to attach SyndDB

    Args:
        db_path: Path to SQLite database file
        sequencer_url: URL of sequencer TEE
        **kwargs: Optional config (flush_interval_ms, snapshot_interval)

    Returns:
        SyndDB instance

    Example:
        >>> from synddb import attach
        >>> synddb = attach('app.db', 'http://localhost:8433')
    """
    if kwargs:
        return SyndDB.attach_with_config(db_path, sequencer_url, **kwargs)
    else:
        return SyndDB.attach(db_path, sequencer_url)
