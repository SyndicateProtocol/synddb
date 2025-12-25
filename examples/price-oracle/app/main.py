#!/usr/bin/env python3
"""Price Oracle CLI

Command-line interface for the price oracle application.
"""

import logging
import os
import sys
import time
from typing import Optional

import click

# Add parent to path for imports
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from app.api import CoinGeckoAPI, CoinMarketCapAPI, MockAPI
from app.api.mock import create_consistent_mock_apis, create_divergent_mock_apis
from app.oracle import PriceOracle
from app.schema import init_database, add_tracked_asset, get_tracked_assets


def setup_logging(verbose: bool) -> None:
    """Configure logging."""
    level = logging.DEBUG if verbose else logging.INFO
    logging.basicConfig(
        level=level,
        format="%(asctime)s [%(levelname)s] %(name)s: %(message)s",
        datefmt="%Y-%m-%d %H:%M:%S",
    )


@click.group()
@click.option("-v", "--verbose", is_flag=True, help="Enable verbose logging")
@click.option(
    "--db",
    "db_path",
    default="price-oracle.db",
    envvar="DATABASE_PATH",
    help="Path to SQLite database",
)
@click.option(
    "--sequencer-url",
    envvar="SEQUENCER_URL",
    help="SyndDB sequencer URL",
)
@click.pass_context
def cli(ctx: click.Context, verbose: bool, db_path: str, sequencer_url: Optional[str]) -> None:
    """Price Oracle - Fetch and store cryptocurrency prices.

    This application demonstrates SyndDB custom validator rules by fetching
    prices from multiple sources and letting validators check consistency.
    """
    setup_logging(verbose)
    ctx.ensure_object(dict)
    ctx.obj["db_path"] = db_path
    ctx.obj["sequencer_url"] = sequencer_url
    ctx.obj["verbose"] = verbose


@cli.command()
@click.option("--mock", is_flag=True, help="Use mock APIs instead of real ones")
@click.option("--divergent", is_flag=True, help="Use divergent mock APIs (for testing rejection)")
@click.option("--divergence", default=5.0, help="Divergence percentage for mock APIs")
@click.option("--coingecko-key", envvar="COINGECKO_API_KEY", help="CoinGecko API key")
@click.option("--cmc-key", envvar="CMC_API_KEY", help="CoinMarketCap API key")
@click.pass_context
def fetch(
    ctx: click.Context,
    mock: bool,
    divergent: bool,
    divergence: float,
    coingecko_key: Optional[str],
    cmc_key: Optional[str],
) -> None:
    """Fetch prices from all configured sources.

    Fetches prices for all tracked assets and stores them in the database.
    If SyndDB is configured, changesets are published to the sequencer.
    """
    db_path = ctx.obj["db_path"]
    sequencer_url = ctx.obj["sequencer_url"]

    # Create APIs
    if mock or divergent:
        if divergent:
            click.echo(f"Using divergent mock APIs ({divergence}% difference)")
            apis = list(create_divergent_mock_apis(divergence))
        else:
            click.echo("Using consistent mock APIs")
            apis = list(create_consistent_mock_apis(0.5))
    else:
        apis = []
        if coingecko_key or True:  # CoinGecko free tier works without key
            apis.append(CoinGeckoAPI(api_key=coingecko_key))
        if cmc_key:
            apis.append(CoinMarketCapAPI(api_key=cmc_key))

        if not apis:
            click.echo("Error: No APIs configured. Use --mock or provide API keys.", err=True)
            sys.exit(1)

    # Run oracle
    with PriceOracle(db_path, apis, sequencer_url) as oracle:
        results = oracle.fetch_and_store()

        click.echo(f"\nFetched prices from {len(results)} sources:")
        for source, prices in results.items():
            click.echo(f"\n  {source}:")
            for p in prices:
                click.echo(f"    {p.asset}: ${p.price:,.2f}")


@cli.command()
@click.argument("asset", default="BTC")
@click.pass_context
def compare(ctx: click.Context, asset: str) -> None:
    """Compare prices from different sources for an asset."""
    db_path = ctx.obj["db_path"]
    sequencer_url = ctx.obj["sequencer_url"]

    with PriceOracle(db_path, [], sequencer_url) as oracle:
        comparison = oracle.get_price_comparison(asset)

        if not comparison:
            click.echo(f"No comparison data available for {asset}")
            return

        click.echo(f"\nPrice comparison for {asset}:")
        click.echo("-" * 40)

        for source, data in comparison["sources"].items():
            click.echo(f"  {source:20} ${data['price']:>12,.2f}")

        click.echo("-" * 40)
        click.echo(f"  {'Average':20} ${comparison['avg_price']:>12,.2f}")
        click.echo(f"  {'Min':20} ${comparison['min_price']:>12,.2f}")
        click.echo(f"  {'Max':20} ${comparison['max_price']:>12,.2f}")
        click.echo(f"  {'Difference':20} {comparison['difference_bps']:>12} bps ({comparison['difference_pct']:.2f}%)")


@cli.command()
@click.option("-n", "--limit", default=20, help="Number of prices to show")
@click.option("--asset", help="Filter by asset")
@click.pass_context
def prices(ctx: click.Context, limit: int, asset: Optional[str]) -> None:
    """Show recent prices from the database."""
    db_path = ctx.obj["db_path"]
    sequencer_url = ctx.obj["sequencer_url"]

    with PriceOracle(db_path, [], sequencer_url) as oracle:
        recent = oracle.get_latest_prices(asset)[:limit]

        if not recent:
            click.echo("No prices in database")
            return

        click.echo(f"\nRecent prices (limit {limit}):")
        click.echo("-" * 70)
        click.echo(f"{'Asset':8} {'Source':15} {'Price':>15} {'Timestamp':>20}")
        click.echo("-" * 70)

        for p in recent:
            ts = time.strftime("%Y-%m-%d %H:%M:%S", time.localtime(p["timestamp"]))
            click.echo(f"{p['asset']:8} {p['source']:15} ${p['price']:>14,.2f} {ts:>20}")


@cli.command()
@click.pass_context
def assets(ctx: click.Context) -> None:
    """List tracked assets."""
    db_path = ctx.obj["db_path"]

    conn = init_database(db_path)
    tracked = get_tracked_assets(conn)
    conn.close()

    if not tracked:
        click.echo("No tracked assets")
        return

    click.echo("\nTracked assets:")
    for a in tracked:
        click.echo(f"  {a['display_name']:8} ({a['symbol']})")


@cli.command("add-asset")
@click.argument("symbol")
@click.argument("display_name")
@click.pass_context
def add_asset(ctx: click.Context, symbol: str, display_name: str) -> None:
    """Add an asset to track.

    SYMBOL is the API symbol (e.g., 'bitcoin', 'ethereum').
    DISPLAY_NAME is the ticker (e.g., 'BTC', 'ETH').
    """
    db_path = ctx.obj["db_path"]

    conn = init_database(db_path)
    add_tracked_asset(conn, symbol, display_name)
    conn.close()

    click.echo(f"Added {display_name} ({symbol}) to tracked assets")


@cli.command()
@click.pass_context
def init(ctx: click.Context) -> None:
    """Initialize the database schema."""
    db_path = ctx.obj["db_path"]

    conn = init_database(db_path)
    conn.close()

    click.echo(f"Initialized database at {db_path}")


@cli.command()
@click.pass_context
def snapshot(ctx: click.Context) -> None:
    """Create a database snapshot via SyndDB."""
    db_path = ctx.obj["db_path"]
    sequencer_url = ctx.obj["sequencer_url"]

    if not sequencer_url:
        click.echo("Error: --sequencer-url required for snapshots", err=True)
        sys.exit(1)

    with PriceOracle(db_path, [], sequencer_url) as oracle:
        size = oracle.create_snapshot()
        if size:
            click.echo(f"Created snapshot: {size:,} bytes")
        else:
            click.echo("Failed to create snapshot")


if __name__ == "__main__":
    cli()
