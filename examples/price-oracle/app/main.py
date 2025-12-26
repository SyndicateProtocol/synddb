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
from app.bridge import (
    create_price_update_message,
    create_batch_price_update_message,
    process_pending_price_requests,
    get_outbound_message_stats,
    get_inbound_message_stats,
    PriceUpdate,
)


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


# ============================================================
# Bridge Commands
# ============================================================


@cli.command("push-prices")
@click.option("--mock", is_flag=True, help="Use mock APIs")
@click.option("--batch", is_flag=True, help="Send as batch update")
@click.pass_context
def push_prices(ctx: click.Context, mock: bool, batch: bool) -> None:
    """Push current prices to the PriceOracle contract (outbound message).

    Fetches prices and creates outbound messages in message_log.
    The sequencer will pick these up and submit to the Bridge.
    """
    db_path = ctx.obj["db_path"]
    sequencer_url = ctx.obj["sequencer_url"]

    # Create mock APIs for fetching
    if mock:
        apis = list(create_consistent_mock_apis(0.5))
    else:
        apis = [CoinGeckoAPI()]

    with PriceOracle(db_path, apis, sequencer_url) as oracle:
        # Fetch prices first
        results = oracle.fetch_and_store()

        if not results:
            click.echo("No prices fetched")
            return

        # Collect all prices for message creation
        conn = init_database(db_path)

        if batch:
            # Create a single batch message
            updates = []
            for source, prices in results.items():
                for p in prices:
                    updates.append(
                        PriceUpdate(
                            asset=p.asset,
                            price=int(p.price * 10**8),
                            timestamp=p.timestamp,
                        )
                    )

            msg_id = create_batch_price_update_message(conn, updates, "push_command")
            if msg_id:
                click.echo(f"Created batch price update message (id={msg_id}) with {len(updates)} prices")
            else:
                click.echo("Batch message already exists (duplicate)")
        else:
            # Create individual messages
            created = 0
            for source, prices in results.items():
                for p in prices:
                    msg_id = create_price_update_message(
                        conn,
                        asset=p.asset,
                        price=p.price,
                        timestamp=p.timestamp,
                        trigger_event="push_command",
                    )
                    if msg_id:
                        created += 1
                        click.echo(f"Created price update for {p.asset}: ${p.price:,.2f}")

            click.echo(f"\nCreated {created} price update messages")

        conn.close()


@cli.command("process-requests")
@click.option("--mock", is_flag=True, help="Use mock APIs for price lookup")
@click.pass_context
def process_requests(ctx: click.Context, mock: bool) -> None:
    """Process pending price requests from the chain (pull model).

    Reads unprocessed requests from inbound_message_log and creates
    response messages in message_log.
    """
    db_path = ctx.obj["db_path"]

    conn = init_database(db_path)

    # Define a function to get current prices
    def get_price(asset: str) -> Optional[tuple[float, int]]:
        """Get the latest price for an asset from the database."""
        cursor = conn.execute(
            """
            SELECT price, timestamp FROM prices
            WHERE asset = ?
            ORDER BY timestamp DESC
            LIMIT 1
            """,
            (asset,),
        )
        row = cursor.fetchone()
        if row:
            return (row[0], row[1])
        return None

    processed = process_pending_price_requests(conn, get_price)
    conn.close()

    if processed > 0:
        click.echo(f"Processed {processed} price requests")
    else:
        click.echo("No pending price requests")


@cli.command("message-stats")
@click.pass_context
def message_stats(ctx: click.Context) -> None:
    """Show message queue statistics."""
    db_path = ctx.obj["db_path"]

    conn = init_database(db_path)

    outbound = get_outbound_message_stats(conn)
    inbound = get_inbound_message_stats(conn)

    conn.close()

    click.echo("\nOutbound Messages (message_log):")
    click.echo(f"  Pending:   {outbound['pending']:>6}")
    click.echo(f"  Submitted: {outbound['submitted']:>6}")
    click.echo(f"  Confirmed: {outbound['confirmed']:>6}")
    click.echo(f"  Failed:    {outbound['failed']:>6}")
    click.echo(f"  Total:     {outbound['total']:>6}")

    click.echo("\nInbound Messages (inbound_message_log):")
    click.echo(f"  Pending:   {inbound['pending']:>6}")
    click.echo(f"  Processed: {inbound['processed']:>6}")
    click.echo(f"  Total:     {inbound['total']:>6}")


@cli.command("watch")
@click.option("--contract", required=True, help="PriceOracle contract address")
@click.option("--rpc-url", required=True, help="Ethereum RPC URL")
@click.option("--start-block", default=0, help="Block to start scanning from")
@click.option("--poll-interval", default=5, help="Seconds between polls")
@click.pass_context
def watch(
    ctx: click.Context,
    contract: str,
    rpc_url: str,
    start_block: int,
    poll_interval: int,
) -> None:
    """Watch for PriceRequested events from the contract.

    Starts a chain monitor that listens for price request events
    and inserts them into inbound_message_log.
    """
    db_path = ctx.obj["db_path"]

    try:
        from app.chain_monitor import (
            ChainMonitorConfig,
            PriceRequestHandler,
            PollingChainMonitor,
        )
    except ImportError as e:
        click.echo(f"Error: {e}", err=True)
        click.echo("Install web3 with: pip install web3", err=True)
        sys.exit(1)

    # Initialize database
    conn = init_database(db_path)
    conn.close()

    config = ChainMonitorConfig(
        rpc_url=rpc_url,
        contract_address=contract,
        start_block=start_block,
        poll_interval=poll_interval,
    )

    handler = PriceRequestHandler(db_path)

    click.echo(f"Starting chain monitor for {contract}")
    click.echo(f"RPC: {rpc_url}")
    click.echo(f"Starting from block: {start_block or 'latest'}")
    click.echo("Press Ctrl+C to stop\n")

    monitor = PollingChainMonitor(config, handler)

    import signal

    def handle_sigint(sig, frame):
        click.echo("\nShutting down...")
        sys.exit(0)

    signal.signal(signal.SIGINT, handle_sigint)

    monitor.run()


@cli.command("run-daemon")
@click.option("--mock", is_flag=True, help="Use mock APIs")
@click.option("--interval", default=60, help="Fetch interval in seconds")
@click.option("--push/--no-push", default=False, help="Push prices to contract")
@click.option("--contract", help="PriceOracle contract address (for push)")
@click.pass_context
def run_daemon(
    ctx: click.Context,
    mock: bool,
    interval: int,
    push: bool,
    contract: Optional[str],
) -> None:
    """Run as a daemon, periodically fetching prices.

    Optionally pushes prices to the contract and processes incoming requests.
    """
    db_path = ctx.obj["db_path"]
    sequencer_url = ctx.obj["sequencer_url"]

    if push and not contract:
        click.echo("Error: --contract required when using --push", err=True)
        sys.exit(1)

    # Create APIs
    if mock:
        apis = list(create_consistent_mock_apis(0.5))
    else:
        apis = [CoinGeckoAPI()]

    click.echo(f"Starting price oracle daemon")
    click.echo(f"  Fetch interval: {interval}s")
    click.echo(f"  Push to contract: {push}")
    if sequencer_url:
        click.echo(f"  Sequencer: {sequencer_url}")
    click.echo("Press Ctrl+C to stop\n")

    import signal

    running = True

    def handle_sigint(sig, frame):
        nonlocal running
        click.echo("\nShutting down...")
        running = False

    signal.signal(signal.SIGINT, handle_sigint)

    with PriceOracle(db_path, apis, sequencer_url) as oracle:
        while running:
            try:
                # Fetch and store prices
                results = oracle.fetch_and_store()
                total_prices = sum(len(p) for p in results.values())
                click.echo(f"Fetched {total_prices} prices from {len(results)} sources")

                # Push to contract if enabled
                if push and results:
                    conn = init_database(db_path)
                    updates = []
                    for source, prices in results.items():
                        for p in prices:
                            updates.append(
                                PriceUpdate(
                                    asset=p.asset,
                                    price=int(p.price * 10**8),
                                    timestamp=p.timestamp,
                                )
                            )
                    if updates:
                        msg_id = create_batch_price_update_message(
                            conn, updates, "daemon_push"
                        )
                        if msg_id:
                            click.echo(f"Created outbound message with {len(updates)} prices")
                    conn.close()

                # Process any pending price requests
                conn = init_database(db_path)

                def get_price(asset: str) -> Optional[tuple[float, int]]:
                    cursor = conn.execute(
                        "SELECT price, timestamp FROM prices WHERE asset = ? ORDER BY timestamp DESC LIMIT 1",
                        (asset,),
                    )
                    row = cursor.fetchone()
                    return (row[0], row[1]) if row else None

                processed = process_pending_price_requests(conn, get_price)
                if processed > 0:
                    click.echo(f"Processed {processed} price requests")
                conn.close()

            except Exception as e:
                logging.error(f"Error in daemon loop: {e}")

            # Sleep until next interval
            time.sleep(interval)


if __name__ == "__main__":
    cli()
