"""Data models for the price oracle."""

from typing import Optional

from pydantic import BaseModel


class Asset(BaseModel):
    """A tracked cryptocurrency asset."""

    symbol: str
    display_name: str
    active: bool = True


class PriceRecord(BaseModel):
    """A single price record from a data source."""

    asset: str
    source: str
    price: float
    volume_24h: Optional[float] = None
    market_cap: Optional[float] = None
    timestamp: int


class PriceComparison(BaseModel):
    """Result of comparing prices across sources."""

    asset: str
    avg_price: float
    min_price: float
    max_price: float
    difference_bps: int
    difference_pct: float
    sources: list[str]


class Snapshot(BaseModel):
    """An aggregated price snapshot."""

    asset: str
    avg_price: float
    min_price: float
    max_price: float
    price_diff_bps: int
    sources_count: int
    timestamp: int


class FetchedPrice(BaseModel):
    """Price data fetched from a source."""

    asset: str
    price: float
    volume_24h: Optional[float] = None
    market_cap: Optional[float] = None
