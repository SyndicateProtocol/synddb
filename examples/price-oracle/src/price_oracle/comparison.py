"""Price comparison and validation logic."""

from .models import PriceComparison, PriceRecord


class PriceDivergenceError(Exception):
    """Raised when prices diverge beyond the allowed threshold."""

    def __init__(self, asset: str, actual_bps: int, max_bps: int):
        self.asset = asset
        self.actual_bps = actual_bps
        self.max_bps = max_bps
        super().__init__(
            f"PRICE_DIVERGENCE: {asset} prices diverge by {actual_bps} bps (max: {max_bps})"
        )


class InsufficientSourcesError(Exception):
    """Raised when there aren't enough sources to compare."""

    def __init__(self, asset: str, count: int):
        self.asset = asset
        self.count = count
        super().__init__(
            f"INSUFFICIENT_SOURCES: {asset} has only {count} source(s), need at least 2"
        )


def compare_prices(
    prices: list[PriceRecord], max_diff_bps: int = 100
) -> PriceComparison:
    """
    Compare prices across sources and validate divergence.

    Args:
        prices: List of price records from different sources
        max_diff_bps: Maximum allowed difference in basis points (100 = 1%)

    Returns:
        PriceComparison with aggregated statistics

    Raises:
        InsufficientSourcesError: If fewer than 2 sources provided
        PriceDivergenceError: If price divergence exceeds threshold
    """
    if len(prices) < 2:
        asset = prices[0].asset if prices else "unknown"
        raise InsufficientSourcesError(asset, len(prices))

    asset = prices[0].asset
    price_values = [p.price for p in prices]

    min_price = min(price_values)
    max_price = max(price_values)
    avg_price = sum(price_values) / len(price_values)

    difference_bps = int(((max_price - min_price) / avg_price) * 10000)
    difference_pct = ((max_price - min_price) / avg_price) * 100

    if difference_bps > max_diff_bps:
        raise PriceDivergenceError(asset, difference_bps, max_diff_bps)

    sources = [p.source for p in prices]

    return PriceComparison(
        asset=asset,
        avg_price=avg_price,
        min_price=min_price,
        max_price=max_price,
        difference_bps=difference_bps,
        difference_pct=difference_pct,
        sources=sources,
    )
