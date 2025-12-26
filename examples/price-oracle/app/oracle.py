"""Price Oracle Core Logic

This module contains the core logic for the price oracle:
- Fetching prices from multiple sources
- Storing prices in the database
- Integration with SyndDB for changeset capture
"""

import logging
import sqlite3
import sys
import time
from pathlib import Path
from typing import Optional

from .api.base import PriceAPI, PriceData
from .schema import init_database, get_tracked_assets, SCHEMA

logger = logging.getLogger(__name__)


class PriceOracle:
    """Main price oracle class.

    Coordinates fetching prices from multiple APIs and storing them
    in a SyndDB-enabled database.
    """

    def __init__(
        self,
        db_path: str,
        apis: list[PriceAPI],
        sequencer_url: Optional[str] = None,
    ):
        """Initialize the price oracle.

        Args:
            db_path: Path to SQLite database
            apis: List of price APIs to use
            sequencer_url: SyndDB sequencer URL (None for standalone mode)
        """
        self.db_path = db_path
        self.apis = apis
        self.sequencer_url = sequencer_url
        self._conn: Optional[sqlite3.Connection] = None
        self._synddb = None

    def start(self) -> None:
        """Start the oracle (initialize database and SyndDB)."""
        logger.info(f"Initializing database at {self.db_path}")

        if self.sequencer_url:
            logger.info(f"Attaching SyndDB to sequencer at {self.sequencer_url}")
            try:
                # Add the bindings directory to path
                bindings_path = Path(__file__).parent.parent.parent.parent / "crates" / "synddb-client" / "bindings" / "python"
                sys.path.insert(0, str(bindings_path))

                from synddb import SyndDB
                self._synddb = SyndDB.attach(self.db_path, self.sequencer_url)
                logger.info("SyndDB attached successfully")

                # Initialize schema through SyndDB so it's captured
                logger.info("Initializing schema through SyndDB")
                self._synddb.execute_batch(SCHEMA)

                # Create a snapshot after schema initialization
                # This ensures validators can restore the full database state
                # (SQLite session extension doesn't capture DDL, only DML)
                logger.info("Creating snapshot after schema initialization")
                snapshot_size = self._synddb.snapshot()
                logger.info(f"Initial snapshot created: {snapshot_size} bytes")
                self._synddb.publish()

                # Also open a read-only connection for queries
                self._conn = sqlite3.connect(self.db_path)
            except ImportError as e:
                logger.warning(f"SyndDB bindings not available: {e}")
                logger.warning("Running in standalone mode (no changeset capture)")
                self._conn = init_database(self.db_path)
            except RuntimeError as e:
                logger.error(f"Failed to attach SyndDB: {e}")
                raise
        else:
            # Standalone mode
            self._conn = init_database(self.db_path)

    def stop(self) -> None:
        """Stop the oracle (cleanup resources)."""
        if self._synddb:
            logger.info("Detaching SyndDB")
            self._synddb.detach()
            self._synddb = None

        if self._conn:
            self._conn.close()
            self._conn = None

    def fetch_and_store(self) -> dict[str, list[PriceData]]:
        """Fetch prices from all APIs and store in database.

        Returns:
            Dict mapping source name to list of PriceData stored
        """
        if not self._conn:
            raise RuntimeError("Oracle not started")

        # Get tracked assets
        assets = get_tracked_assets(self._conn)
        symbols = [a["symbol"] for a in assets]

        if not symbols:
            logger.warning("No tracked assets configured")
            return {}

        logger.info(f"Fetching prices for {len(symbols)} assets from {len(self.apis)} sources")

        results: dict[str, list[PriceData]] = {}

        for api in self.apis:
            try:
                prices = api.get_prices(symbols)
                if prices:
                    self._store_prices(prices)
                    results[api.source_name] = prices
                    logger.info(f"Stored {len(prices)} prices from {api.source_name}")
                else:
                    logger.warning(f"No prices returned from {api.source_name}")
            except Exception as e:
                logger.error(f"Error fetching from {api.source_name}: {e}")

        # Publish changesets if SyndDB is attached
        if self._synddb:
            try:
                self._synddb.publish()
                logger.debug("Published changesets to sequencer")
            except RuntimeError as e:
                logger.error(f"Failed to publish changesets: {e}")

        return results

    def _store_prices(self, prices: list[PriceData]) -> None:
        """Store prices in the database.

        When SyndDB is attached, uses the monitored connection so changes
        are captured and published. Otherwise falls back to sqlite3.
        """
        if self._synddb:
            # Use SyndDB's monitored connection for writes
            self._synddb.begin()
            try:
                for price in prices:
                    # Build SQL with inline values (SyndDB execute doesn't support params yet)
                    volume = price.volume_24h if price.volume_24h is not None else "NULL"
                    market_cap = price.market_cap if price.market_cap is not None else "NULL"
                    sql = f"""
                        INSERT INTO prices (asset, source, price, timestamp, volume_24h, market_cap)
                        VALUES ('{price.asset}', '{price.source}', {price.price}, {price.timestamp}, {volume}, {market_cap})
                    """
                    self._synddb.execute(sql)
                self._synddb.commit()
            except Exception:
                self._synddb.rollback()
                raise
        else:
            # Fallback to sqlite3 connection (standalone mode)
            if not self._conn:
                raise RuntimeError("Oracle not started")

            for price in prices:
                self._conn.execute(
                    """
                    INSERT INTO prices (asset, source, price, timestamp, volume_24h, market_cap)
                    VALUES (?, ?, ?, ?, ?, ?)
                    """,
                    (
                        price.asset,
                        price.source,
                        price.price,
                        price.timestamp,
                        price.volume_24h,
                        price.market_cap,
                    ),
                )
            self._conn.commit()

    def get_latest_prices(self, asset: Optional[str] = None) -> list[dict]:
        """Get latest prices from the database.

        Args:
            asset: Optional asset filter

        Returns:
            List of price records
        """
        if not self._conn:
            raise RuntimeError("Oracle not started")

        if asset:
            cursor = self._conn.execute(
                """
                SELECT asset, source, price, timestamp, volume_24h, market_cap
                FROM prices
                WHERE asset = ?
                ORDER BY timestamp DESC
                LIMIT 10
                """,
                (asset,),
            )
        else:
            cursor = self._conn.execute(
                """
                SELECT asset, source, price, timestamp, volume_24h, market_cap
                FROM prices
                ORDER BY timestamp DESC
                LIMIT 50
                """
            )

        return [
            {
                "asset": row[0],
                "source": row[1],
                "price": row[2],
                "timestamp": row[3],
                "volume_24h": row[4],
                "market_cap": row[5],
            }
            for row in cursor.fetchall()
        ]

    def get_price_comparison(self, asset: str) -> Optional[dict]:
        """Compare latest prices from different sources.

        Args:
            asset: Asset to compare

        Returns:
            Comparison dict with prices, difference, etc.
        """
        if not self._conn:
            raise RuntimeError("Oracle not started")

        cursor = self._conn.execute(
            """
            SELECT source, price, timestamp
            FROM prices
            WHERE asset = ?
            ORDER BY timestamp DESC
            """,
            (asset,),
        )

        # Get latest price per source
        source_prices: dict[str, tuple[float, int]] = {}
        for row in cursor.fetchall():
            source = row[0]
            if source not in source_prices:
                source_prices[source] = (row[1], row[2])

        if len(source_prices) < 2:
            return None

        prices = list(source_prices.values())
        price_values = [p[0] for p in prices]
        avg_price = sum(price_values) / len(price_values)
        min_price = min(price_values)
        max_price = max(price_values)
        diff_bps = int(((max_price - min_price) / avg_price) * 10000)

        return {
            "asset": asset,
            "sources": {k: {"price": v[0], "timestamp": v[1]} for k, v in source_prices.items()},
            "avg_price": avg_price,
            "min_price": min_price,
            "max_price": max_price,
            "difference_bps": diff_bps,
            "difference_pct": diff_bps / 100,
        }

    def create_snapshot(self) -> Optional[int]:
        """Create a database snapshot via SyndDB.

        Returns:
            Snapshot size in bytes, or None if SyndDB not attached
        """
        if not self._synddb:
            logger.warning("SyndDB not attached, cannot create snapshot")
            return None

        try:
            size = self._synddb.snapshot()
            logger.info(f"Created snapshot: {size} bytes")
            return size
        except RuntimeError as e:
            logger.error(f"Failed to create snapshot: {e}")
            return None

    def __enter__(self):
        """Context manager entry."""
        self.start()
        return self

    def __exit__(self, exc_type, exc_val, exc_tb):
        """Context manager exit."""
        self.stop()
