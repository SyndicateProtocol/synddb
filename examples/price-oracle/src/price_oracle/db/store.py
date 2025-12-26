"""SQLite database store for price data."""

import sqlite3
import threading
import time
from typing import Optional

from ..models import Asset, PriceRecord, Snapshot

SCHEMA = """
CREATE TABLE IF NOT EXISTS assets (
    symbol TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    active INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE IF NOT EXISTS prices (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    asset TEXT NOT NULL,
    source TEXT NOT NULL,
    price REAL NOT NULL,
    volume_24h REAL,
    market_cap REAL,
    timestamp INTEGER NOT NULL,
    FOREIGN KEY (asset) REFERENCES assets(symbol)
);

CREATE INDEX IF NOT EXISTS idx_prices_asset_timestamp ON prices(asset, timestamp DESC);
CREATE INDEX IF NOT EXISTS idx_prices_source ON prices(source);

CREATE TABLE IF NOT EXISTS snapshots (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    asset TEXT NOT NULL,
    avg_price REAL NOT NULL,
    min_price REAL NOT NULL,
    max_price REAL NOT NULL,
    price_diff_bps INTEGER NOT NULL,
    sources_count INTEGER NOT NULL,
    timestamp INTEGER NOT NULL,
    FOREIGN KEY (asset) REFERENCES assets(symbol)
);

CREATE INDEX IF NOT EXISTS idx_snapshots_asset_timestamp ON snapshots(asset, timestamp DESC);

CREATE TABLE IF NOT EXISTS bridge_messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    message_id TEXT UNIQUE,
    asset TEXT NOT NULL,
    price_scaled TEXT NOT NULL,
    timestamp INTEGER NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    tx_hash TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
"""


class PriceStore:
    """Thread-safe SQLite store for price data."""

    def __init__(self, db_path: str = ":memory:"):
        self.db_path = db_path
        self._local = threading.local()

    def _get_conn(self) -> sqlite3.Connection:
        if not hasattr(self._local, "conn"):
            self._local.conn = sqlite3.connect(self.db_path, check_same_thread=False)
            self._local.conn.row_factory = sqlite3.Row
        return self._local.conn

    def init_schema(self) -> None:
        """Initialize the database schema."""
        conn = self._get_conn()
        conn.executescript(SCHEMA)
        self._insert_default_assets(conn)
        conn.commit()

    def _insert_default_assets(self, conn: sqlite3.Connection) -> None:
        """Insert default assets (BTC, ETH)."""
        conn.execute(
            "INSERT OR IGNORE INTO assets (symbol, display_name, active) VALUES (?, ?, ?)",
            ("bitcoin", "BTC", 1),
        )
        conn.execute(
            "INSERT OR IGNORE INTO assets (symbol, display_name, active) VALUES (?, ?, ?)",
            ("ethereum", "ETH", 1),
        )

    def get_active_assets(self) -> list[Asset]:
        """Get all active assets."""
        conn = self._get_conn()
        cursor = conn.execute(
            "SELECT symbol, display_name, active FROM assets WHERE active = 1"
        )
        return [
            Asset(symbol=row["symbol"], display_name=row["display_name"], active=True)
            for row in cursor.fetchall()
        ]

    def get_all_assets(self) -> list[Asset]:
        """Get all assets including inactive ones."""
        conn = self._get_conn()
        cursor = conn.execute("SELECT symbol, display_name, active FROM assets")
        return [
            Asset(
                symbol=row["symbol"],
                display_name=row["display_name"],
                active=bool(row["active"]),
            )
            for row in cursor.fetchall()
        ]

    def add_asset(self, symbol: str, display_name: str) -> None:
        """Add a new asset to track."""
        conn = self._get_conn()
        conn.execute(
            "INSERT INTO assets (symbol, display_name, active) VALUES (?, ?, 1)",
            (symbol, display_name),
        )
        conn.commit()

    def set_asset_active(self, symbol: str, active: bool) -> None:
        """Set asset active/inactive status."""
        conn = self._get_conn()
        conn.execute(
            "UPDATE assets SET active = ? WHERE symbol = ?", (1 if active else 0, symbol)
        )
        conn.commit()

    def insert_price(self, record: PriceRecord) -> None:
        """Insert a price record."""
        conn = self._get_conn()
        conn.execute(
            """INSERT INTO prices (asset, source, price, volume_24h, market_cap, timestamp)
               VALUES (?, ?, ?, ?, ?, ?)""",
            (
                record.asset,
                record.source,
                record.price,
                record.volume_24h,
                record.market_cap,
                record.timestamp,
            ),
        )
        conn.commit()

    def get_prices(
        self, asset: Optional[str] = None, limit: int = 100
    ) -> list[PriceRecord]:
        """Get recent prices, optionally filtered by asset."""
        conn = self._get_conn()
        if asset:
            cursor = conn.execute(
                """SELECT asset, source, price, volume_24h, market_cap, timestamp
                   FROM prices WHERE asset = ? ORDER BY timestamp DESC LIMIT ?""",
                (asset, limit),
            )
        else:
            cursor = conn.execute(
                """SELECT asset, source, price, volume_24h, market_cap, timestamp
                   FROM prices ORDER BY timestamp DESC LIMIT ?""",
                (limit,),
            )
        return [
            PriceRecord(
                asset=row["asset"],
                source=row["source"],
                price=row["price"],
                volume_24h=row["volume_24h"],
                market_cap=row["market_cap"],
                timestamp=row["timestamp"],
            )
            for row in cursor.fetchall()
        ]

    def get_latest_price_per_source(self, asset: str) -> list[PriceRecord]:
        """Get the most recent price from each source for an asset."""
        conn = self._get_conn()
        cursor = conn.execute(
            """SELECT p.asset, p.source, p.price, p.volume_24h, p.market_cap, p.timestamp
               FROM prices p
               INNER JOIN (
                   SELECT source, MAX(timestamp) as max_ts
                   FROM prices
                   WHERE asset = ?
                   GROUP BY source
               ) latest ON p.source = latest.source AND p.timestamp = latest.max_ts
               WHERE p.asset = ?""",
            (asset, asset),
        )
        return [
            PriceRecord(
                asset=row["asset"],
                source=row["source"],
                price=row["price"],
                volume_24h=row["volume_24h"],
                market_cap=row["market_cap"],
                timestamp=row["timestamp"],
            )
            for row in cursor.fetchall()
        ]

    def insert_snapshot(self, snapshot: Snapshot) -> None:
        """Insert an aggregated snapshot."""
        conn = self._get_conn()
        conn.execute(
            """INSERT INTO snapshots (asset, avg_price, min_price, max_price,
                                       price_diff_bps, sources_count, timestamp)
               VALUES (?, ?, ?, ?, ?, ?, ?)""",
            (
                snapshot.asset,
                snapshot.avg_price,
                snapshot.min_price,
                snapshot.max_price,
                snapshot.price_diff_bps,
                snapshot.sources_count,
                snapshot.timestamp,
            ),
        )
        conn.commit()

    def get_snapshots(self, asset: Optional[str] = None, limit: int = 100) -> list[Snapshot]:
        """Get recent snapshots."""
        conn = self._get_conn()
        if asset:
            cursor = conn.execute(
                """SELECT asset, avg_price, min_price, max_price, price_diff_bps,
                          sources_count, timestamp
                   FROM snapshots WHERE asset = ? ORDER BY timestamp DESC LIMIT ?""",
                (asset, limit),
            )
        else:
            cursor = conn.execute(
                """SELECT asset, avg_price, min_price, max_price, price_diff_bps,
                          sources_count, timestamp
                   FROM snapshots ORDER BY timestamp DESC LIMIT ?""",
                (limit,),
            )
        return [
            Snapshot(
                asset=row["asset"],
                avg_price=row["avg_price"],
                min_price=row["min_price"],
                max_price=row["max_price"],
                price_diff_bps=row["price_diff_bps"],
                sources_count=row["sources_count"],
                timestamp=row["timestamp"],
            )
            for row in cursor.fetchall()
        ]

    def record_bridge_message(
        self, message_id: str, asset: str, price_scaled: str, timestamp: int
    ) -> None:
        """Record a bridge message submission."""
        conn = self._get_conn()
        now = int(time.time())
        conn.execute(
            """INSERT INTO bridge_messages
               (message_id, asset, price_scaled, timestamp, status, created_at, updated_at)
               VALUES (?, ?, ?, ?, 'pending', ?, ?)""",
            (message_id, asset, price_scaled, timestamp, now, now),
        )
        conn.commit()

    def update_bridge_message_status(
        self, message_id: str, status: str, tx_hash: Optional[str] = None
    ) -> None:
        """Update bridge message status."""
        conn = self._get_conn()
        now = int(time.time())
        if tx_hash:
            conn.execute(
                "UPDATE bridge_messages SET status = ?, tx_hash = ?, updated_at = ? WHERE message_id = ?",
                (status, tx_hash, now, message_id),
            )
        else:
            conn.execute(
                "UPDATE bridge_messages SET status = ?, updated_at = ? WHERE message_id = ?",
                (status, now, message_id),
            )
        conn.commit()
