#!/usr/bin/env python3
"""Price Oracle HTTP Server

Flask-based HTTP server for the price oracle, providing:
- REST API for price queries
- Manual fetch trigger endpoint
- Health check endpoint
"""

import logging
import os
import sys
from typing import Optional

from flask import Flask, jsonify, request

# Add parent to path for imports
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from app.api import MockAPI
from app.api.mock import create_consistent_mock_apis, create_divergent_mock_apis
from app.oracle import PriceOracle
from app.schema import get_tracked_assets

logger = logging.getLogger(__name__)

# Flask app
app = Flask(__name__)

# Global oracle instance (set by run_server)
_oracle: Optional[PriceOracle] = None


def get_oracle() -> PriceOracle:
    """Get the oracle instance."""
    if _oracle is None:
        raise RuntimeError("Oracle not initialized")
    return _oracle


@app.route("/health")
def health():
    """Health check endpoint."""
    return jsonify({"status": "ok"})


@app.route("/prices")
def list_prices():
    """Get recent prices.

    Query params:
        - asset: Filter by asset symbol (e.g., 'BTC')
        - limit: Max number of results (default 50)
    """
    oracle = get_oracle()
    asset = request.args.get("asset")
    limit = request.args.get("limit", 50, type=int)

    prices = oracle.get_latest_prices(asset)[:limit]
    return jsonify({"prices": prices, "count": len(prices)})


@app.route("/prices/<asset>")
def get_price(asset: str):
    """Get latest prices for a specific asset."""
    oracle = get_oracle()
    prices = oracle.get_latest_prices(asset)

    if not prices:
        return jsonify({"error": f"No prices found for {asset}"}), 404

    return jsonify({"asset": asset, "prices": prices})


@app.route("/compare/<asset>")
def compare_prices(asset: str):
    """Compare prices from different sources for an asset."""
    oracle = get_oracle()
    comparison = oracle.get_price_comparison(asset)

    if not comparison:
        return jsonify({"error": f"Not enough data to compare for {asset}"}), 404

    return jsonify(comparison)


@app.route("/fetch", methods=["POST"])
def trigger_fetch():
    """Manually trigger a price fetch.

    This endpoint fetches prices from all configured APIs and stores them.
    """
    oracle = get_oracle()
    results = oracle.fetch_and_store()

    response = {"fetched": {}}
    for source, prices in results.items():
        response["fetched"][source] = [
            {"asset": p.asset, "price": p.price}
            for p in prices
        ]

    return jsonify(response)


@app.route("/assets")
def list_assets():
    """List tracked assets."""
    oracle = get_oracle()
    assets = get_tracked_assets(oracle._conn)
    return jsonify({"assets": assets})


@app.route("/snapshot", methods=["POST"])
def create_snapshot():
    """Create a database snapshot via SyndDB."""
    oracle = get_oracle()
    size = oracle.create_snapshot()

    if size is None:
        return jsonify({"error": "SyndDB not configured"}), 400

    return jsonify({"snapshot_size": size})


def run_server(
    db_path: str = "price-oracle.db",
    sequencer_url: Optional[str] = None,
    host: str = "0.0.0.0",
    port: int = 5000,
    use_mock: bool = False,
    mock_divergent: bool = False,
    mock_divergence: float = 5.0,
    debug: bool = False,
) -> None:
    """Run the HTTP server.

    Args:
        db_path: Path to SQLite database
        sequencer_url: SyndDB sequencer URL
        host: Host to bind to
        port: Port to listen on
        use_mock: Use mock APIs
        mock_divergent: Use divergent mock APIs
        mock_divergence: Divergence percentage for mock APIs
        debug: Enable Flask debug mode
    """
    global _oracle

    # Create APIs
    if use_mock or mock_divergent:
        if mock_divergent:
            apis = list(create_divergent_mock_apis(mock_divergence))
        else:
            apis = list(create_consistent_mock_apis(0.5))
    else:
        # In production, configure real APIs
        apis = []

    # Create and start oracle
    _oracle = PriceOracle(db_path, apis, sequencer_url)
    _oracle.start()

    try:
        logger.info(f"Starting HTTP server on {host}:{port}")
        app.run(host=host, port=port, debug=debug)
    finally:
        _oracle.stop()


if __name__ == "__main__":
    import argparse

    parser = argparse.ArgumentParser(description="Price Oracle HTTP Server")
    parser.add_argument("--db", default="price-oracle.db", help="Database path")
    parser.add_argument("--sequencer-url", help="SyndDB sequencer URL")
    parser.add_argument("--host", default="0.0.0.0", help="Host to bind to")
    parser.add_argument("--port", type=int, default=5000, help="Port to listen on")
    parser.add_argument("--mock", action="store_true", help="Use mock APIs")
    parser.add_argument("--divergent", action="store_true", help="Use divergent mock APIs")
    parser.add_argument("--divergence", type=float, default=5.0, help="Divergence percentage")
    parser.add_argument("--debug", action="store_true", help="Enable debug mode")

    args = parser.parse_args()

    logging.basicConfig(level=logging.INFO)

    run_server(
        db_path=args.db,
        sequencer_url=args.sequencer_url,
        host=args.host,
        port=args.port,
        use_mock=args.mock,
        mock_divergent=args.divergent,
        mock_divergence=args.divergence,
        debug=args.debug,
    )
