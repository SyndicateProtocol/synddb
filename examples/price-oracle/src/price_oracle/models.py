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


class PriceStats(BaseModel):
    """Statistical summary of prices over a time window."""

    asset: str
    window_seconds: int
    count: int
    avg_price: float
    min_price: float
    max_price: float
    stddev: float
    volatility_pct: float  # stddev / avg * 100
    from_timestamp: int
    to_timestamp: int


class OHLCCandle(BaseModel):
    """OHLC candlestick data for a time interval."""

    asset: str
    interval: str  # e.g., "1m", "5m", "1h", "1d"
    open_price: float
    high_price: float
    low_price: float
    close_price: float
    volume: Optional[float] = None
    timestamp: int  # candle start time
