"""FastAPI HTTP endpoints for the price oracle."""

import time
from typing import Optional

from fastapi import FastAPI, HTTPException, Query
from pydantic import BaseModel

from .comparison import InsufficientSourcesError, PriceDivergenceError, compare_prices
from .config import FetchMode, settings
from .db import PriceStore
from .models import Asset, PriceComparison, PriceRecord, Snapshot
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
