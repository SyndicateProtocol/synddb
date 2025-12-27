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

    # Optionally force immediate publish (auto-publishes every second)
    synddb.publish_changeset()

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
        # Workspace root target (correct path for cargo workspace)
        os.path.join(os.path.dirname(__file__), '..', '..', '..', '..', 'target', 'release', lib_name),
        os.path.join(os.path.dirname(__file__), '..', '..', '..', '..', 'target', 'debug', lib_name),
        # Crate-level target (legacy, may not exist)
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
    INVALID_URL = 7

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

_lib.synddb_publish_changeset.argtypes = [ctypes.POINTER(_SyndDBHandle)]
_lib.synddb_publish_changeset.restype = ctypes.c_int

_lib.synddb_publish_snapshot.argtypes = [
    ctypes.POINTER(_SyndDBHandle),
    ctypes.POINTER(ctypes.c_size_t)  # out_size (optional)
]
_lib.synddb_publish_snapshot.restype = ctypes.c_int

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
        snapshot_interval: int = 100
    ) -> 'SyndDB':
        """
        Attach SyndDB with custom configuration

        Args:
            db_path: Path to SQLite database file
            sequencer_url: URL of sequencer TEE
            flush_interval_ms: Milliseconds between automatic publishes (must be > 0, default: 1000)
            snapshot_interval: Changesets between snapshots (must be > 0, default: 100)

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

    def publish_changeset(self):
        """
        Publish all pending changesets to the sequencer immediately

        Changesets are automatically published on a timer. Use this to force
        immediate publication for low-latency or high-value changes.

        Raises:
            RuntimeError: If publish fails

        Example:
            >>> synddb.publish_changeset()
        """
        if not self._handle:
            raise RuntimeError("SyndDB handle already detached")

        result = _lib.synddb_publish_changeset(self._handle)

        if result != SyndDBError.SUCCESS:
            error_msg = _lib.synddb_last_error()
            error_str = error_msg.decode('utf-8') if error_msg else "Unknown error"
            raise RuntimeError(f"Failed to publish changeset (error {result}): {error_str}")

    def snapshot(self) -> int:
        """
        Create and publish a snapshot to the sequencer.

        This creates a complete database snapshot (schema + data) and sends it
        to the sequencer. Use this after schema changes (CREATE TABLE, etc.)
        since DDL is NOT captured in changesets.

        This is consistent with publish_changeset() for changesets:
        - publish_changeset() - sends pending changesets to sequencer
        - snapshot() - creates and sends snapshot to sequencer

        When to use:
        - After CREATE TABLE, ALTER TABLE, or other DDL statements
        - To create periodic recovery checkpoints
        - Before major migrations

        Returns:
            Size of snapshot in bytes

        Raises:
            RuntimeError: If snapshot creation or publishing fails

        Example:
            >>> # After creating schema
            >>> synddb.execute_batch("CREATE TABLE users (id INTEGER PRIMARY KEY)")
            >>> size = synddb.snapshot()  # Creates AND publishes
            >>> print(f"Published snapshot: {size} bytes")
        """
        if not self._handle:
            raise RuntimeError("SyndDB handle already detached")

        size = ctypes.c_size_t()
        result = _lib.synddb_publish_snapshot(self._handle, ctypes.byref(size))

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


# ============================================================================
# Message Passing API
# ============================================================================

import json
import urllib.request
import urllib.error


class MessageClient:
    """Simple client for interacting with the sequencer's message API.

    This provides the simplest possible DX for receiving inbound messages
    from the blockchain and sending responses.

    Example:
        >>> from synddb import MessageClient
        >>> client = MessageClient('http://localhost:8433')
        >>>
        >>> # Get pending price requests
        >>> messages = client.get_messages(type='price_request')
        >>> for msg in messages:
        ...     print(f"Request for {msg['payload']['asset']}")
        ...     # Process and respond...
        ...     client.ack(msg['id'])
    """

    def __init__(self, sequencer_url: str):
        """Initialize the message client.

        Args:
            sequencer_url: URL of the sequencer (e.g., 'http://localhost:8433')
        """
        self.base_url = sequencer_url.rstrip('/')

    def get_messages(
        self,
        type: Optional[str] = None,
        after_id: int = 0,
        limit: int = 100,
        pending_only: bool = True,
    ) -> list[dict]:
        """Get inbound messages from the sequencer.

        Args:
            type: Filter by message type (e.g., 'price_request', 'deposit')
            after_id: Get messages with ID greater than this
            limit: Maximum messages to return (default: 100, max: 1000)
            pending_only: Only return unacknowledged messages (default: True)

        Returns:
            List of message dicts with keys: id, message_id, type, payload,
            sender, tx_hash, block_number, confirmations, timestamp, acknowledged

        Example:
            >>> messages = client.get_messages(type='price_request')
            >>> for msg in messages:
            ...     print(f"Asset: {msg['payload']['asset']}")
        """
        params = {
            'after_id': after_id,
            'limit': min(limit, 1000),
            'pending_only': 'true' if pending_only else 'false',
        }
        if type:
            params['type'] = type

        query = '&'.join(f"{k}={v}" for k, v in params.items())
        url = f"{self.base_url}/messages/inbound?{query}"

        try:
            with urllib.request.urlopen(url, timeout=10) as response:
                data = json.loads(response.read().decode('utf-8'))
                return data.get('messages', [])
        except urllib.error.HTTPError as e:
            raise RuntimeError(f"Failed to get messages: {e.code} {e.reason}")
        except urllib.error.URLError as e:
            raise RuntimeError(f"Failed to connect to sequencer: {e.reason}")

    def get_message(self, message_id: int) -> dict:
        """Get a specific message by ID.

        Args:
            message_id: The message ID

        Returns:
            Message dict

        Raises:
            RuntimeError: If message not found or request fails
        """
        url = f"{self.base_url}/messages/inbound/{message_id}"

        try:
            with urllib.request.urlopen(url, timeout=10) as response:
                return json.loads(response.read().decode('utf-8'))
        except urllib.error.HTTPError as e:
            if e.code == 404:
                raise RuntimeError(f"Message {message_id} not found")
            raise RuntimeError(f"Failed to get message: {e.code} {e.reason}")
        except urllib.error.URLError as e:
            raise RuntimeError(f"Failed to connect to sequencer: {e.reason}")

    def ack(self, message_id: int, processed: bool = True, note: Optional[str] = None) -> bool:
        """Acknowledge an inbound message.

        Call this after processing a message to mark it as handled.
        Acknowledged messages won't be returned by get_messages() with pending_only=True.

        Args:
            message_id: The message ID to acknowledge
            processed: Whether the message was successfully processed (default: True)
            note: Optional note about processing

        Returns:
            True if message was acknowledged, False if already acknowledged

        Example:
            >>> messages = client.get_messages()
            >>> for msg in messages:
            ...     # Process...
            ...     client.ack(msg['id'])
        """
        url = f"{self.base_url}/messages/inbound/{message_id}/ack"
        data = json.dumps({'processed': processed, 'note': note}).encode('utf-8')

        req = urllib.request.Request(url, data=data, method='POST')
        req.add_header('Content-Type', 'application/json')

        try:
            with urllib.request.urlopen(req, timeout=10) as response:
                result = json.loads(response.read().decode('utf-8'))
                return result.get('acknowledged', False)
        except urllib.error.HTTPError as e:
            raise RuntimeError(f"Failed to acknowledge message: {e.code} {e.reason}")
        except urllib.error.URLError as e:
            raise RuntimeError(f"Failed to connect to sequencer: {e.reason}")

    def get_outbound_status(self, message_id: int) -> dict:
        """Get the status of an outbound message.

        Check if a message you wrote to message_log has been submitted
        to the blockchain.

        Args:
            message_id: The message ID from your message_log table

        Returns:
            Status dict with keys: id, message_type, status, tx_hash, confirmations,
            error, first_seen_at, updated_at
        """
        url = f"{self.base_url}/messages/outbound/{message_id}/status"

        try:
            with urllib.request.urlopen(url, timeout=10) as response:
                return json.loads(response.read().decode('utf-8'))
        except urllib.error.HTTPError as e:
            raise RuntimeError(f"Failed to get outbound status: {e.code} {e.reason}")
        except urllib.error.URLError as e:
            raise RuntimeError(f"Failed to connect to sequencer: {e.reason}")

    def outbound_stats(self) -> dict:
        """Get outbound message statistics.

        Returns:
            Stats dict with keys: total, pending, queued, submitting, submitted,
            confirmed, failed, monitor_active
        """
        url = f"{self.base_url}/messages/outbound/stats"

        try:
            with urllib.request.urlopen(url, timeout=10) as response:
                return json.loads(response.read().decode('utf-8'))
        except urllib.error.HTTPError as e:
            raise RuntimeError(f"Failed to get outbound stats: {e.code} {e.reason}")
        except urllib.error.URLError as e:
            raise RuntimeError(f"Failed to connect to sequencer: {e.reason}")

    def stats(self) -> dict:
        """Get message queue statistics.

        Returns:
            Stats dict with keys: total, pending, acknowledged, max_size
        """
        url = f"{self.base_url}/messages/inbound/stats"

        try:
            with urllib.request.urlopen(url, timeout=10) as response:
                return json.loads(response.read().decode('utf-8'))
        except urllib.error.HTTPError as e:
            raise RuntimeError(f"Failed to get stats: {e.code} {e.reason}")
        except urllib.error.URLError as e:
            raise RuntimeError(f"Failed to connect to sequencer: {e.reason}")

    def push(
        self,
        message_id: str,
        message_type: str,
        payload: dict,
        sender: str,
        tx_hash: str,
        block_number: int,
        confirmations: int = 0,
    ) -> dict:
        """Push a new inbound message to the sequencer queue.

        Called by chain monitors to submit blockchain events to the queue.
        The message will be assigned a sequence ID and made available for
        apps to retrieve via get_messages().

        Args:
            message_id: Message ID from blockchain (e.g., requestId)
            message_type: Type of message (e.g., 'price_request')
            payload: Message payload as dict
            sender: Sender address on blockchain
            tx_hash: Transaction hash where event was emitted
            block_number: Block number where event was emitted
            confirmations: Number of confirmations (default: 0)

        Returns:
            Dict with 'id' (sequencer-assigned) and 'message_id'

        Example:
            >>> client.push(
            ...     message_id='0xabc123',
            ...     message_type='price_request',
            ...     payload={'asset': 'BTC', 'max_age': 300},
            ...     sender='0x1234...',
            ...     tx_hash='0xdef456...',
            ...     block_number=12345,
            ... )
            {'id': 1, 'message_id': '0xabc123'}
        """
        url = f"{self.base_url}/messages/inbound"
        data = json.dumps({
            'message_id': message_id,
            'type': message_type,
            'payload': payload,
            'sender': sender,
            'tx_hash': tx_hash,
            'block_number': block_number,
            'confirmations': confirmations,
        }).encode('utf-8')

        req = urllib.request.Request(url, data=data, method='POST')
        req.add_header('Content-Type', 'application/json')

        try:
            with urllib.request.urlopen(req, timeout=10) as response:
                return json.loads(response.read().decode('utf-8'))
        except urllib.error.HTTPError as e:
            raise RuntimeError(f"Failed to push message: {e.code} {e.reason}")
        except urllib.error.URLError as e:
            raise RuntimeError(f"Failed to connect to sequencer: {e.reason}")
