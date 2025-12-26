"""CLI commands for the price oracle."""

import asyncio
import logging
import time
from typing import Optional

import click

from .comparison import InsufficientSourcesError, PriceDivergenceError, compare_prices
from .config import FetchMode, settings
from .db import PriceStore
from .models import PriceRecord, Snapshot
from .sources import CoinGeckoSource, CoinMarketCapSource, MockMode, MockSource, PriceSource

logger = logging.getLogger(__name__)


def get_store() -> PriceStore:
    """Get the database store."""
    return PriceStore(settings.database_path)


def create_sources(mode: FetchMode) -> list[PriceSource]:
    """Create price sources based on mode."""
    if mode == FetchMode.MOCK:
        return [
            MockSource("mock1", MockMode.CONSISTENT, seed=42),
            MockSource("mock2", MockMode.CONSISTENT, seed=43),
        ]
    elif mode == FetchMode.DIVERGENT:
        return [
            MockSource("mock1", MockMode.CONSISTENT, seed=42),
            MockSource("mock2", MockMode.DIVERGENT, seed=43),
        ]
    else:
        sources: list[PriceSource] = [CoinGeckoSource(settings.coingecko_api_key)]
        if settings.cmc_api_key:
            sources.append(CoinMarketCapSource(settings.cmc_api_key))
        return sources


async def do_fetch(store: PriceStore, sources: list[PriceSource]) -> int:
    """Fetch prices from all sources and store them."""
    assets = store.get_active_assets()
    asset_symbols = [a.symbol for a in assets]
    total_count = 0
    now = int(time.time())

    for source in sources:
        try:
            prices = await source.fetch_prices(asset_symbols)
            for price in prices:
                record = PriceRecord(
                    asset=price.asset,
                    source=source.name,
                    price=price.price,
                    volume_24h=price.volume_24h,
                    market_cap=price.market_cap,
                    timestamp=now,
                )
                store.insert_price(record)
                total_count += 1
            click.echo(f"Fetched {len(prices)} prices from {source.name}")
        except Exception as e:
            click.echo(f"Error fetching from {source.name}: {e}", err=True)

    return total_count


@click.group()
@click.option("-v", "--verbose", count=True, help="Increase verbosity")
@click.option("--db", "database_path", default=None, help="Database path")
@click.option("--mode", type=click.Choice(["real", "mock", "divergent"]), default=None)
@click.pass_context
def main(ctx: click.Context, verbose: int, database_path: Optional[str], mode: Optional[str]) -> None:
    """Price Oracle - Multi-source cryptocurrency price aggregation."""
    logging.basicConfig(
        level=logging.DEBUG if verbose > 1 else logging.INFO if verbose else logging.WARNING,
        format="%(asctime)s - %(name)s - %(levelname)s - %(message)s",
    )

    if database_path:
        settings.database_path = database_path
    if mode:
        settings.fetch_mode = FetchMode(mode)

    ctx.ensure_object(dict)
    ctx.obj["store"] = get_store()


@main.command()
@click.pass_context
def init(ctx: click.Context) -> None:
    """Initialize the database schema."""
    store: PriceStore = ctx.obj["store"]
    store.init_schema()
    click.echo("Database initialized successfully")


@main.command()
@click.pass_context
def fetch(ctx: click.Context) -> None:
    """Fetch prices from all sources."""
    store: PriceStore = ctx.obj["store"]
    sources = create_sources(settings.fetch_mode)

    count = asyncio.run(do_fetch(store, sources))
    click.echo(f"Total: {count} prices stored")


@main.command()
@click.option("--asset", default=None, help="Filter by asset")
@click.option("--limit", default=10, help="Number of records to show")
@click.pass_context
def prices(ctx: click.Context, asset: Optional[str], limit: int) -> None:
    """Show recent prices."""
    store: PriceStore = ctx.obj["store"]
    records = store.get_prices(asset=asset, limit=limit)

    if not records:
        click.echo("No prices found")
        return

    click.echo(f"{'Asset':<12} {'Source':<15} {'Price':>15} {'Timestamp'}")
    click.echo("-" * 60)
    for r in records:
        click.echo(f"{r.asset:<12} {r.source:<15} ${r.price:>14,.2f} {r.timestamp}")


@main.command()
@click.argument("asset")
@click.pass_context
def compare(ctx: click.Context, asset: str) -> None:
    """Compare prices across sources for an asset."""
    store: PriceStore = ctx.obj["store"]
    records = store.get_latest_price_per_source(asset)

    if not records:
        click.echo(f"No prices found for {asset}")
        return

    try:
        result = compare_prices(records, settings.max_price_difference_bps)
        click.echo(f"\nPrice Comparison for {asset.upper()}")
        click.echo("=" * 40)
        click.echo(f"Average Price:  ${result.avg_price:,.2f}")
        click.echo(f"Min Price:      ${result.min_price:,.2f}")
        click.echo(f"Max Price:      ${result.max_price:,.2f}")
        click.echo(f"Difference:     {result.difference_bps} bps ({result.difference_pct:.2f}%)")
        click.echo(f"Sources:        {', '.join(result.sources)}")
        click.echo(f"\nStatus: VALID (within {settings.max_price_difference_bps} bps tolerance)")

    except InsufficientSourcesError as e:
        click.echo(f"Cannot compare: {e}", err=True)

    except PriceDivergenceError as e:
        click.echo(f"\nPrice Comparison for {asset.upper()}")
        click.echo("=" * 40)
        for r in records:
            click.echo(f"  {r.source}: ${r.price:,.2f}")
        click.echo(f"\nStatus: INVALID - {e}", err=True)


@main.command()
@click.pass_context
def assets(ctx: click.Context) -> None:
    """List tracked assets."""
    store: PriceStore = ctx.obj["store"]
    asset_list = store.get_all_assets()

    click.echo(f"{'Symbol':<15} {'Display':<10} {'Status'}")
    click.echo("-" * 35)
    for a in asset_list:
        status = "Active" if a.active else "Inactive"
        click.echo(f"{a.symbol:<15} {a.display_name:<10} {status}")


@main.command("add-asset")
@click.argument("symbol")
@click.argument("display_name")
@click.pass_context
def add_asset(ctx: click.Context, symbol: str, display_name: str) -> None:
    """Add a new asset to track."""
    store: PriceStore = ctx.obj["store"]
    store.add_asset(symbol, display_name)
    click.echo(f"Added asset: {symbol} ({display_name})")


@main.command()
@click.pass_context
def snapshot(ctx: click.Context) -> None:
    """Create snapshots for all active assets."""
    store: PriceStore = ctx.obj["store"]
    assets = store.get_active_assets()
    now = int(time.time())
    count = 0

    for asset in assets:
        records = store.get_latest_price_per_source(asset.symbol)
        if len(records) < 2:
            click.echo(f"Skipping {asset.symbol}: insufficient sources")
            continue

        try:
            result = compare_prices(records, settings.max_price_difference_bps)
            snapshot_record = Snapshot(
                asset=result.asset,
                avg_price=result.avg_price,
                min_price=result.min_price,
                max_price=result.max_price,
                price_diff_bps=result.difference_bps,
                sources_count=len(result.sources),
                timestamp=now,
            )
            store.insert_snapshot(snapshot_record)
            count += 1
            click.echo(f"Snapshot created for {asset.symbol}: ${result.avg_price:,.2f}")

        except PriceDivergenceError as e:
            click.echo(f"Skipping {asset.symbol}: {e}", err=True)

    click.echo(f"\nTotal snapshots created: {count}")


@main.command("run-daemon")
@click.pass_context
def run_daemon(ctx: click.Context) -> None:
    """Run continuous fetcher with periodic updates."""
    store: PriceStore = ctx.obj["store"]
    sources = create_sources(settings.fetch_mode)

    click.echo(f"Starting daemon (interval: {settings.fetch_interval_seconds}s)")
    click.echo("Press Ctrl+C to stop\n")

    cycle_count = 0

    async def daemon_loop() -> None:
        nonlocal cycle_count
        while True:
            try:
                cycle_count += 1
                click.echo(f"--- Cycle {cycle_count} ---")

                await do_fetch(store, sources)

                if cycle_count % settings.snapshot_interval == 0:
                    click.echo("Creating snapshots...")
                    assets = store.get_active_assets()
                    now = int(time.time())

                    for asset in assets:
                        records = store.get_latest_price_per_source(asset.symbol)
                        if len(records) >= 2:
                            try:
                                result = compare_prices(records, settings.max_price_difference_bps)
                                snapshot_record = Snapshot(
                                    asset=result.asset,
                                    avg_price=result.avg_price,
                                    min_price=result.min_price,
                                    max_price=result.max_price,
                                    price_diff_bps=result.difference_bps,
                                    sources_count=len(result.sources),
                                    timestamp=now,
                                )
                                store.insert_snapshot(snapshot_record)
                            except PriceDivergenceError:
                                pass

                await asyncio.sleep(settings.fetch_interval_seconds)

            except KeyboardInterrupt:
                break

    try:
        asyncio.run(daemon_loop())
    except KeyboardInterrupt:
        click.echo("\nDaemon stopped")


@main.command("push-prices")
@click.option("--asset", default=None, help="Push only this asset")
@click.pass_context
def push_prices(ctx: click.Context, asset: Optional[str]) -> None:
    """Push current prices to Bridge validator."""
    if not settings.bridge_validator_url:
        click.echo("Error: BRIDGE_VALIDATOR_URL not configured", err=True)
        return

    from .bridge import BridgeClient

    store: PriceStore = ctx.obj["store"]
    client = BridgeClient(settings.bridge_validator_url, settings.bridge_domain)

    async def push_all() -> None:
        assets = store.get_active_assets()
        if asset:
            assets = [a for a in assets if a.symbol == asset]

        for a in assets:
            records = store.get_latest_price_per_source(a.symbol)
            if not records:
                click.echo(f"No prices for {a.symbol}")
                continue

            try:
                result = compare_prices(records, settings.max_price_difference_bps)
                msg_id = await client.push_price(a.symbol, result.avg_price)
                if msg_id:
                    click.echo(f"Pushed {a.symbol}: ${result.avg_price:,.2f} -> {msg_id}")
                    store.record_bridge_message(
                        msg_id, a.symbol, str(int(result.avg_price * 10**18)), int(time.time())
                    )
                else:
                    click.echo(f"Failed to push {a.symbol}", err=True)

            except (InsufficientSourcesError, PriceDivergenceError) as e:
                click.echo(f"Skipping {a.symbol}: {e}", err=True)

        await client.close()

    asyncio.run(push_all())


@main.command()
@click.pass_context
def serve(ctx: click.Context) -> None:
    """Start the HTTP API server."""
    import uvicorn

    from .api import create_app

    store: PriceStore = ctx.obj["store"]
    app = create_app(store)

    click.echo(f"Starting HTTP server on {settings.http_host}:{settings.http_port}")
    uvicorn.run(app, host=settings.http_host, port=settings.http_port, log_level="info")


if __name__ == "__main__":
    main()
