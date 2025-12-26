"""FastAPI HTTP endpoints for the price oracle."""

import time
from typing import Optional

from fastapi import FastAPI, HTTPException, Query
from pydantic import BaseModel

from .comparison import InsufficientSourcesError, PriceDivergenceError, compare_prices
from .config import FetchMode, settings
from .db import PriceStore
from .models import Asset, OHLCCandle, PriceComparison, PriceRecord, PriceStats, Snapshot
from .sources import CoinGeckoSource, CoinMarketCapSource, MockMode, MockSource, PriceSource


class HealthResponse(BaseModel):
    healthy: bool
    mode: str


class FetchResponse(BaseModel):
    count: int
    sources: list[str]


class ErrorResponse(BaseModel):
    error: str
    code: str


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


def create_app(store: Optional[PriceStore] = None) -> FastAPI:
    """Create the FastAPI application."""
    app = FastAPI(
        title="Price Oracle API",
        description="Multi-source cryptocurrency price aggregation with Bridge integration",
        version="0.1.0",
    )

    if store is None:
        store = PriceStore(settings.database_path)
        store.init_schema()

    sources = create_sources(settings.fetch_mode)

    @app.get("/health", response_model=HealthResponse)
    async def health() -> HealthResponse:
        """Health check endpoint."""
        return HealthResponse(healthy=True, mode=settings.fetch_mode.value)

    @app.get("/prices", response_model=list[PriceRecord])
    async def get_prices(
        asset: Optional[str] = Query(None, description="Filter by asset"),
        limit: int = Query(100, ge=1, le=1000, description="Maximum records to return"),
    ) -> list[PriceRecord]:
        """Get recent prices."""
        return store.get_prices(asset=asset, limit=limit)

    @app.get("/prices/{asset}", response_model=list[PriceRecord])
    async def get_asset_prices(asset: str) -> list[PriceRecord]:
        """Get prices for a specific asset."""
        prices = store.get_prices(asset=asset, limit=100)
        if not prices:
            raise HTTPException(status_code=404, detail=f"No prices found for {asset}")
        return prices

    # -------------------------------------------------------------------------
    # Complex Query Endpoints - Demonstrating SQLite analytical capabilities
    # -------------------------------------------------------------------------

    @app.get("/prices/{asset}/history", response_model=list[PriceRecord])
    async def get_price_history(
        asset: str,
        from_ts: int = Query(..., description="Start timestamp (unix seconds)"),
        to_ts: int = Query(..., description="End timestamp (unix seconds)"),
    ) -> list[PriceRecord]:
        """Get price history for an asset within a time range.

        This endpoint demonstrates SQLite's efficient time-range queries.
        In a pure message-passing system, you'd need to:
        - Query each block/event individually, or
        - Build an off-chain indexer
        """
        if from_ts > to_ts:
            raise HTTPException(status_code=400, detail="from_ts must be <= to_ts")
        return store.get_price_history(asset, from_ts, to_ts)

    @app.get("/prices/{asset}/stats", response_model=PriceStats)
    async def get_price_stats(
        asset: str,
        window: str = Query(
            "1h",
            description="Time window: 1m, 5m, 15m, 1h, 4h, 1d",
            regex="^(1m|5m|15m|1h|4h|1d)$",
        ),
    ) -> PriceStats:
        """Get statistical summary of prices over a time window.

        Returns: count, avg, min, max, stddev, volatility_pct

        This single query demonstrates what would require either:
        - Multiple contract calls in pure message-passing, or
        - An off-chain indexer to aggregate historical data
        """
        window_seconds = {
            "1m": 60,
            "5m": 300,
            "15m": 900,
            "1h": 3600,
            "4h": 14400,
            "1d": 86400,
        }.get(window, 3600)

        stats = store.get_price_stats(asset, window_seconds)
        if stats is None:
            raise HTTPException(
                status_code=404,
                detail=f"No prices found for {asset} in the last {window}",
            )
        return stats

    @app.get("/prices/{asset}/ohlc", response_model=list[OHLCCandle])
    async def get_ohlc(
        asset: str,
        interval: str = Query(
            "1h",
            description="Candle interval: 1m, 5m, 15m, 1h, 4h, 1d",
            regex="^(1m|5m|15m|1h|4h|1d)$",
        ),
        limit: int = Query(24, ge=1, le=100, description="Number of candles"),
    ) -> list[OHLCCandle]:
        """Get OHLC candlestick data for charting.

        This query demonstrates SQLite's ability to:
        - Bucket data by time intervals
        - Compute open/close via subqueries
        - Aggregate within each bucket

        In pure message-passing, you'd need to store individual price events
        on-chain (expensive) or build an off-chain indexer.
        """
        candles = store.get_ohlc(asset, interval, limit)
        if not candles:
            raise HTTPException(
                status_code=404,
                detail=f"No OHLC data for {asset} at {interval} interval",
            )
        return candles

    @app.get("/compare/{asset}", response_model=PriceComparison)
    async def compare_asset(asset: str) -> PriceComparison:
        """Compare prices across sources for an asset."""
        records = store.get_latest_price_per_source(asset)
        if not records:
            raise HTTPException(status_code=404, detail=f"No prices found for {asset}")

        try:
            return compare_prices(records, settings.max_price_difference_bps)
        except InsufficientSourcesError as e:
            raise HTTPException(
                status_code=400,
                detail={"error": str(e), "code": "INSUFFICIENT_SOURCES"},
            )
        except PriceDivergenceError as e:
            raise HTTPException(
                status_code=422,
                detail={
                    "error": str(e),
                    "code": "PRICE_DIVERGENCE",
                    "actual_bps": e.actual_bps,
                    "max_bps": e.max_bps,
                },
            )

    @app.post("/fetch", response_model=FetchResponse)
    async def trigger_fetch() -> FetchResponse:
        """Trigger a manual price fetch from all sources."""
        assets = store.get_active_assets()
        asset_symbols = [a.symbol for a in assets]
        total_count = 0
        now = int(time.time())
        source_names = []

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
                source_names.append(source.name)
            except Exception:
                pass

        return FetchResponse(count=total_count, sources=source_names)

    @app.get("/assets", response_model=list[Asset])
    async def list_assets() -> list[Asset]:
        """List all tracked assets."""
        return store.get_all_assets()

    @app.post("/snapshot", response_model=list[Snapshot])
    async def create_snapshot() -> list[Snapshot]:
        """Create snapshots for all active assets."""
        assets = store.get_active_assets()
        now = int(time.time())
        snapshots = []

        for asset in assets:
            records = store.get_latest_price_per_source(asset.symbol)
            if len(records) < 2:
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
                snapshots.append(snapshot_record)
            except PriceDivergenceError:
                pass

        return snapshots

    @app.get("/snapshots", response_model=list[Snapshot])
    async def get_snapshots(
        asset: Optional[str] = Query(None, description="Filter by asset"),
        limit: int = Query(100, ge=1, le=1000, description="Maximum records to return"),
    ) -> list[Snapshot]:
        """Get recent snapshots."""
        return store.get_snapshots(asset=asset, limit=limit)

    return app
