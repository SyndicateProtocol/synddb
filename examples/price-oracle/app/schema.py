"""Database schema for the price oracle

The schema stores prices from multiple sources, allowing validators to
check consistency without needing API access.
"""

import sqlite3
from pathlib import Path


SCHEMA = """
-- Prices table stores price data from all sources
CREATE TABLE IF NOT EXISTS prices (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    asset TEXT NOT NULL,           -- e.g., 'BTC', 'ETH'
    source TEXT NOT NULL,          -- 'coingecko' or 'coinmarketcap'
    price REAL NOT NULL,           -- Price in USD
    timestamp INTEGER NOT NULL,    -- Unix timestamp when price was fetched
    -- Additional metadata
    volume_24h REAL,               -- 24h trading volume (optional)
    market_cap REAL                -- Market cap (optional)
);

-- Index for efficient queries by asset and time
CREATE INDEX IF NOT EXISTS idx_prices_asset_time ON prices(asset, timestamp DESC);

-- Index for source-based queries
CREATE INDEX IF NOT EXISTS idx_prices_source ON prices(source);

-- Price snapshots table for aggregated data
CREATE TABLE IF NOT EXISTS price_snapshots (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    asset TEXT NOT NULL,
    avg_price REAL NOT NULL,       -- Average price across sources
    min_price REAL NOT NULL,       -- Minimum price
    max_price REAL NOT NULL,       -- Maximum price
    price_diff_bps INTEGER,        -- Difference in basis points
    sources_count INTEGER NOT NULL, -- Number of sources used
    timestamp INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_snapshots_asset_time ON price_snapshots(asset, timestamp DESC);

-- Assets being tracked
CREATE TABLE IF NOT EXISTS tracked_assets (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    symbol TEXT UNIQUE NOT NULL,   -- e.g., 'bitcoin', 'ethereum' (API format)
    display_name TEXT NOT NULL,    -- e.g., 'BTC', 'ETH' (for display)
    active INTEGER DEFAULT 1       -- Whether to fetch this asset
);

-- Insert default tracked assets
INSERT OR IGNORE INTO tracked_assets (symbol, display_name, active) VALUES
    ('bitcoin', 'BTC', 1),
    ('ethereum', 'ETH', 1);

-- ============================================================
-- BRIDGE MESSAGE TABLES
-- These tables enable bidirectional communication with the
-- PriceOracle smart contract via the SyndDB Bridge.
-- ============================================================

-- Outbound messages: Price updates pushed to the contract
-- The sequencer reads from this table and submits to Bridge.sol
CREATE TABLE IF NOT EXISTS message_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    message_type TEXT NOT NULL,         -- 'price_update', 'batch_price_update', 'price_response'
    payload TEXT NOT NULL,              -- JSON: {asset, price, timestamp} or array for batch
    idempotency_key TEXT UNIQUE,        -- Prevents duplicate messages
    status TEXT DEFAULT 'pending',      -- 'pending', 'submitted', 'confirmed', 'failed'
    created_at INTEGER DEFAULT (unixepoch()),
    -- Audit fields
    trigger_event TEXT,                 -- What caused this message (e.g., 'scheduled_update', 'price_request')
    trigger_id TEXT,                    -- Reference to causing record (e.g., request_id)
    -- Status tracking
    submitted_at INTEGER,
    tx_hash TEXT,
    error TEXT
);

CREATE INDEX IF NOT EXISTS idx_message_log_status ON message_log(status);
CREATE INDEX IF NOT EXISTS idx_message_log_type ON message_log(message_type);

-- Inbound messages: Price requests from the contract
-- The chain monitor listens for PriceRequested events and inserts here
CREATE TABLE IF NOT EXISTS inbound_message_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    message_id TEXT UNIQUE NOT NULL,    -- requestId from the PriceRequested event
    message_type TEXT NOT NULL,         -- 'price_request'
    payload TEXT NOT NULL,              -- JSON: {asset, requester, max_age, block_number, tx_hash}
    block_number INTEGER NOT NULL,
    tx_hash TEXT NOT NULL,
    log_index INTEGER,
    processed INTEGER NOT NULL DEFAULT 0,
    processed_at INTEGER,
    created_at INTEGER DEFAULT (unixepoch()),
    -- Response tracking
    response_message_id INTEGER,        -- FK to message_log if we sent a response
    FOREIGN KEY (response_message_id) REFERENCES message_log(id)
);

CREATE INDEX IF NOT EXISTS idx_inbound_processed ON inbound_message_log(processed);
CREATE INDEX IF NOT EXISTS idx_inbound_type ON inbound_message_log(message_type);
"""


def init_database(db_path: str) -> sqlite3.Connection:
    """Initialize the database with the schema.

    Args:
        db_path: Path to the SQLite database file

    Returns:
        sqlite3.Connection: Connection to the initialized database
    """
    # Create directory if needed
    Path(db_path).parent.mkdir(parents=True, exist_ok=True)

    conn = sqlite3.connect(db_path)
    conn.executescript(SCHEMA)
    conn.commit()
    return conn


def get_tracked_assets(conn: sqlite3.Connection) -> list[dict]:
    """Get list of active tracked assets.

    Args:
        conn: Database connection

    Returns:
        List of dicts with 'symbol' and 'display_name' keys
    """
    cursor = conn.execute(
        "SELECT symbol, display_name FROM tracked_assets WHERE active = 1"
    )
    return [{"symbol": row[0], "display_name": row[1]} for row in cursor.fetchall()]


def add_tracked_asset(conn: sqlite3.Connection, symbol: str, display_name: str) -> None:
    """Add a new asset to track.

    Args:
        conn: Database connection
        symbol: API symbol (e.g., 'bitcoin')
        display_name: Display name (e.g., 'BTC')
    """
    conn.execute(
        "INSERT OR REPLACE INTO tracked_assets (symbol, display_name, active) VALUES (?, ?, 1)",
        (symbol, display_name),
    )
    conn.commit()


def remove_tracked_asset(conn: sqlite3.Connection, symbol: str) -> None:
    """Deactivate an asset from tracking.

    Args:
        conn: Database connection
        symbol: API symbol to deactivate
    """
    conn.execute(
        "UPDATE tracked_assets SET active = 0 WHERE symbol = ?", (symbol,)
    )
    conn.commit()


# ============================================================
# Message Log Functions
# ============================================================


def get_pending_outbound_messages(conn: sqlite3.Connection) -> list[dict]:
    """Get all pending outbound messages.

    Returns:
        List of message dicts with id, message_type, payload, etc.
    """
    cursor = conn.execute(
        """
        SELECT id, message_type, payload, idempotency_key, status, created_at,
               trigger_event, trigger_id
        FROM message_log
        WHERE status = 'pending'
        ORDER BY created_at ASC
        """
    )
    return [
        {
            "id": row[0],
            "message_type": row[1],
            "payload": row[2],
            "idempotency_key": row[3],
            "status": row[4],
            "created_at": row[5],
            "trigger_event": row[6],
            "trigger_id": row[7],
        }
        for row in cursor.fetchall()
    ]


def get_unprocessed_inbound_messages(conn: sqlite3.Connection) -> list[dict]:
    """Get all unprocessed inbound messages (price requests).

    Returns:
        List of message dicts with message_id, payload, etc.
    """
    cursor = conn.execute(
        """
        SELECT id, message_id, message_type, payload, block_number, tx_hash, log_index
        FROM inbound_message_log
        WHERE processed = 0
        ORDER BY created_at ASC
        """
    )
    return [
        {
            "id": row[0],
            "message_id": row[1],
            "message_type": row[2],
            "payload": row[3],
            "block_number": row[4],
            "tx_hash": row[5],
            "log_index": row[6],
        }
        for row in cursor.fetchall()
    ]
