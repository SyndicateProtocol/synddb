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
