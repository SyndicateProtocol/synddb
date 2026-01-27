#!/usr/bin/env python3
"""Price Oracle Fetcher Daemon

Periodically fetches prices from all configured APIs and stores them
in the database. Changesets are pushed to SyndDB if configured.
"""

import argparse
import logging
import os
import signal
import sys
import time
from typing import Optional

# Add parent to path for imports
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from app.api import CoinGeckoAPI, CoinMarketCapAPI
from app.api.mock import create_consistent_mock_apis, create_divergent_mock_apis
from app.oracle import PriceOracle

logger = logging.getLogger(__name__)


class FetcherDaemon:
    """Daemon that periodically fetches prices."""

    def __init__(
        self,
        oracle: PriceOracle,
        interval_seconds: int = 60,
        snapshot_interval: int = 0,
    ):
        """Initialize the fetcher daemon.

        Args:
            oracle: PriceOracle instance
            interval_seconds: Seconds between fetch cycles
            snapshot_interval: Create snapshot every N fetch cycles (0 = disabled)
        """
        self.oracle = oracle
        self.interval_seconds = interval_seconds
        self.snapshot_interval = snapshot_interval
        self.running = False
        self.fetch_count = 0

    def start(self) -> None:
        """Start the daemon loop."""
        self.running = True
        logger.info(f"Starting fetcher daemon (interval: {self.interval_seconds}s)")

        while self.running:
            try:
                self._fetch_cycle()
            except KeyboardInterrupt:
                logger.info("Interrupted, shutting down")
                break
            except Exception as e:
                logger.error(f"Fetch cycle error: {e}")

            if self.running:
                time.sleep(self.interval_seconds)

    def stop(self) -> None:
        """Stop the daemon."""
        logger.info("Stopping fetcher daemon")
        self.running = False

    def _fetch_cycle(self) -> None:
        """Execute one fetch cycle."""
        logger.info(f"Starting fetch cycle #{self.fetch_count + 1}")

        start_time = time.time()
        results = self.oracle.fetch_and_store()
        elapsed = time.time() - start_time

        total_prices = sum(len(prices) for prices in results.values())
        logger.info(
            f"Fetch cycle complete: {total_prices} prices from "
            f"{len(results)} sources in {elapsed:.2f}s"
        )

        self.fetch_count += 1

        # Create snapshot if configured
        if self.snapshot_interval > 0 and self.fetch_count % self.snapshot_interval == 0:
            logger.info("Creating scheduled snapshot")
            self.oracle.create_snapshot()


def run_daemon(
    db_path: str = "price-oracle.db",
    sequencer_url: Optional[str] = None,
    interval: int = 60,
    snapshot_interval: int = 0,
    use_mock: bool = False,
    mock_divergent: bool = False,
    mock_divergence: float = 5.0,
    coingecko_key: Optional[str] = None,
    cmc_key: Optional[str] = None,
) -> None:
    """Run the fetcher daemon.

    Args:
        db_path: Path to SQLite database
        sequencer_url: SyndDB sequencer URL
        interval: Seconds between fetch cycles
        snapshot_interval: Create snapshot every N cycles
        use_mock: Use mock APIs
        mock_divergent: Use divergent mock APIs
        mock_divergence: Divergence percentage
        coingecko_key: CoinGecko API key
        cmc_key: CoinMarketCap API key
    """
    # Create APIs
    if use_mock or mock_divergent:
        if mock_divergent:
            logger.info(f"Using divergent mock APIs ({mock_divergence}% difference)")
            apis = list(create_divergent_mock_apis(mock_divergence))
        else:
            logger.info("Using consistent mock APIs")
            apis = list(create_consistent_mock_apis(0.5))
    else:
        apis = []
        # CoinGecko free tier works without key
        apis.append(CoinGeckoAPI(api_key=coingecko_key))
        if cmc_key:
            apis.append(CoinMarketCapAPI(api_key=cmc_key))
        else:
            logger.warning("CMC_API_KEY not set, only using CoinGecko")

    # Create oracle
    oracle = PriceOracle(db_path, apis, sequencer_url)
    daemon = FetcherDaemon(
        oracle,
        interval_seconds=interval,
        snapshot_interval=snapshot_interval,
    )

    # Handle signals for graceful shutdown
    def signal_handler(signum, frame):
        logger.info(f"Received signal {signum}")
        daemon.stop()

    signal.signal(signal.SIGINT, signal_handler)
    signal.signal(signal.SIGTERM, signal_handler)

    # Start
    oracle.start()
    try:
        daemon.start()
    finally:
        oracle.stop()


def main() -> None:
    """Main entry point."""
    parser = argparse.ArgumentParser(
        description="Price Oracle Fetcher Daemon",
        formatter_class=argparse.ArgumentDefaultsHelpFormatter,
    )
    parser.add_argument(
        "--db",
        default=os.environ.get("DATABASE_PATH", "price-oracle.db"),
        help="Database path",
    )
    parser.add_argument(
        "--sequencer-url",
        default=os.environ.get("SEQUENCER_URL"),
        help="SyndDB sequencer URL",
    )
    parser.add_argument(
        "--interval",
        type=int,
        default=60,
        help="Seconds between fetch cycles",
    )
    parser.add_argument(
        "--snapshot-interval",
        type=int,
        default=0,
        help="Create snapshot every N cycles (0 = disabled)",
    )
    parser.add_argument(
        "--mock",
        action="store_true",
        help="Use mock APIs",
    )
    parser.add_argument(
        "--divergent",
        action="store_true",
        help="Use divergent mock APIs (for testing validator rejection)",
    )
    parser.add_argument(
        "--divergence",
        type=float,
        default=5.0,
        help="Divergence percentage for mock APIs",
    )
    parser.add_argument(
        "--coingecko-key",
        default=os.environ.get("COINGECKO_API_KEY"),
        help="CoinGecko API key",
    )
    parser.add_argument(
        "--cmc-key",
        default=os.environ.get("CMC_API_KEY"),
        help="CoinMarketCap API key",
    )
    parser.add_argument(
        "-v", "--verbose",
        action="store_true",
        help="Enable verbose logging",
    )

    args = parser.parse_args()

    # Configure logging
    logging.basicConfig(
        level=logging.DEBUG if args.verbose else logging.INFO,
        format="%(asctime)s [%(levelname)s] %(name)s: %(message)s",
        datefmt="%Y-%m-%d %H:%M:%S",
    )

    run_daemon(
        db_path=args.db,
        sequencer_url=args.sequencer_url,
        interval=args.interval,
        snapshot_interval=args.snapshot_interval,
        use_mock=args.mock,
        mock_divergent=args.divergent,
        mock_divergence=args.divergence,
        coingecko_key=args.coingecko_key,
        cmc_key=args.cmc_key,
    )


if __name__ == "__main__":
    main()
