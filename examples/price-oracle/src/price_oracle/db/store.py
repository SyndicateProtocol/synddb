"""SQLite database store for price data."""

import sqlite3
import threading
import time
from typing import Optional

import math

from ..models import Asset, OHLCCandle, PriceRecord, PriceStats, Snapshot

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

    # -------------------------------------------------------------------------
    # Complex Query Methods - Demonstrating SQLite analytical capabilities
    # -------------------------------------------------------------------------

    def get_price_history(
        self, asset: str, from_ts: int, to_ts: int
    ) -> list[PriceRecord]:
        """Get all prices in a time range.

        This is a simple time-series query that SQLite handles efficiently
        with the (asset, timestamp DESC) index.
        """
        conn = self._get_conn()
        cursor = conn.execute(
            """SELECT asset, source, price, volume_24h, market_cap, timestamp
               FROM prices
               WHERE asset = ? AND timestamp >= ? AND timestamp <= ?
               ORDER BY timestamp ASC""",
            (asset, from_ts, to_ts),
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

    def get_price_stats(self, asset: str, window_seconds: int) -> Optional[PriceStats]:
        """Calculate statistical summary over a time window.

        Computes avg, min, max, count, and standard deviation.
        SQLite doesn't have STDDEV built-in, so we compute it manually
        using the formula: stddev = sqrt(avg(x^2) - avg(x)^2)

        This single query demonstrates what would require either:
        - Multiple contract calls in pure message-passing, or
        - An off-chain indexer to aggregate historical data
        """
        conn = self._get_conn()
        now = int(time.time())
        from_ts = now - window_seconds

        # First get count, avg, min, max and sum of squares for stddev
        cursor = conn.execute(
            """SELECT
                   COUNT(*) as count,
                   AVG(price) as avg_price,
                   MIN(price) as min_price,
                   MAX(price) as max_price,
                   AVG(price * price) as avg_sq,
                   MIN(timestamp) as from_ts,
                   MAX(timestamp) as to_ts
               FROM prices
               WHERE asset = ? AND timestamp >= ?""",
            (asset, from_ts),
        )
        row = cursor.fetchone()

        if row is None or row["count"] == 0:
            return None

        count = row["count"]
        avg_price = row["avg_price"]
        min_price = row["min_price"]
        max_price = row["max_price"]
        avg_sq = row["avg_sq"]

        # Compute standard deviation: sqrt(E[X^2] - E[X]^2)
        variance = avg_sq - (avg_price * avg_price)
        # Handle floating point errors that could make variance slightly negative
        stddev = math.sqrt(max(0, variance))

        # Volatility as percentage of mean
        volatility_pct = (stddev / avg_price * 100) if avg_price > 0 else 0.0

        return PriceStats(
            asset=asset,
            window_seconds=window_seconds,
            count=count,
            avg_price=avg_price,
            min_price=min_price,
            max_price=max_price,
            stddev=stddev,
            volatility_pct=volatility_pct,
            from_timestamp=row["from_ts"] or from_ts,
            to_timestamp=row["to_ts"] or now,
        )

    def get_ohlc(
        self, asset: str, interval: str, limit: int = 24
    ) -> list[OHLCCandle]:
        """Get OHLC candlestick data by interval.

        Supported intervals: 1m, 5m, 15m, 1h, 4h, 1d

        This query demonstrates SQLite's ability to:
        - Bucket data by time intervals
        - Compute open/close via subqueries
        - Aggregate within each bucket

        In pure message-passing, you'd need to:
        - Store individual price events on-chain (expensive)
        - Build an off-chain indexer to compute OHLC
        - Or accept only the latest price (no historical analysis)
        """
        # Parse interval to seconds
        interval_seconds = self._parse_interval(interval)
        if interval_seconds is None:
            return []

        conn = self._get_conn()
        now = int(time.time())
        # Look back enough time to get `limit` candles
        from_ts = now - (interval_seconds * limit * 2)

        # Use a CTE to bucket prices, then aggregate
        # SQLite integer division truncates, giving us the bucket start time
        cursor = conn.execute(
            """WITH bucketed AS (
                   SELECT
                       asset,
                       price,
                       volume_24h,
                       timestamp,
                       (timestamp / ?) * ? as bucket_start
                   FROM prices
                   WHERE asset = ? AND timestamp >= ?
               )
               SELECT
                   asset,
                   bucket_start,
                   (SELECT price FROM bucketed b2
                    WHERE b2.bucket_start = bucketed.bucket_start
                    ORDER BY b2.timestamp ASC LIMIT 1) as open_price,
                   MAX(price) as high_price,
                   MIN(price) as low_price,
                   (SELECT price FROM bucketed b3
                    WHERE b3.bucket_start = bucketed.bucket_start
                    ORDER BY b3.timestamp DESC LIMIT 1) as close_price,
                   SUM(COALESCE(volume_24h, 0)) as volume
               FROM bucketed
               GROUP BY bucket_start
               ORDER BY bucket_start DESC
               LIMIT ?""",
            (interval_seconds, interval_seconds, asset, from_ts, limit),
        )

        return [
            OHLCCandle(
                asset=row["asset"],
                interval=interval,
                open_price=row["open_price"],
                high_price=row["high_price"],
                low_price=row["low_price"],
                close_price=row["close_price"],
                volume=row["volume"] if row["volume"] else None,
                timestamp=row["bucket_start"],
            )
            for row in cursor.fetchall()
        ]

    @staticmethod
    def _parse_interval(interval: str) -> Optional[int]:
        """Parse interval string to seconds."""
        intervals = {
            "1m": 60,
            "5m": 300,
            "15m": 900,
            "1h": 3600,
            "4h": 14400,
            "1d": 86400,
        }
        return intervals.get(interval)
