"""Mock price API for testing

This mock API generates deterministic or random prices for testing
the price oracle and validator without needing real API keys.
"""

import random
import time
from typing import Optional

from .base import PriceAPI, PriceData


# Base prices for common assets (used as reference)
BASE_PRICES = {
    "bitcoin": 50000.0,
    "ethereum": 3000.0,
    "solana": 100.0,
    "cardano": 0.50,
    "polkadot": 7.0,
}


class MockAPI(PriceAPI):
    """Mock price API for testing.

    Generates prices that are:
    - Deterministic when seed is provided
    - Configurable to simulate different scenarios (consistent, divergent)
    - Fast and doesn't require network access
    """

    def __init__(
        self,
        source_suffix: str = "",
        variance_percent: float = 0.5,
        consistent_with: Optional["MockAPI"] = None,
        seed: Optional[int] = None,
    ):
        """Initialize the mock API.

        Args:
            source_suffix: Suffix to add to source name (e.g., '_a', '_b')
            variance_percent: Maximum variance from base price (0-100)
            consistent_with: Another MockAPI to stay consistent with
            seed: Random seed for reproducible results
        """
        self._source_suffix = source_suffix
        self._variance_percent = variance_percent
        self._consistent_with = consistent_with
        self._rng = random.Random(seed)

        # Cache for prices when staying consistent
        self._price_cache: dict[str, float] = {}

    @property
    def source_name(self) -> str:
        return f"mock{self._source_suffix}"

    def _generate_price(self, symbol: str) -> float:
        """Generate a price for the given symbol."""
        base_price = BASE_PRICES.get(symbol, 100.0)

        if self._consistent_with is not None:
            # Get reference price from the other API
            ref_data = self._consistent_with.get_price(symbol)
            if ref_data:
                base_price = ref_data.price

        # Apply variance
        variance = (self._rng.random() * 2 - 1) * (self._variance_percent / 100)
        return base_price * (1 + variance)

    def get_price(self, symbol: str) -> Optional[PriceData]:
        """Get price for a single asset."""
        # Use cached price if available (for consistency within same fetch cycle)
        if symbol in self._price_cache:
            price = self._price_cache[symbol]
        else:
            price = self._generate_price(symbol)
            self._price_cache[symbol] = price

        # Map symbol to display name
        display_name = symbol.upper()[:3] if len(symbol) >= 3 else symbol.upper()

        return PriceData(
            asset=display_name,
            source=self.source_name,
            price=price,
            timestamp=int(time.time()),
            volume_24h=price * self._rng.uniform(1000000, 10000000),
            market_cap=price * self._rng.uniform(100000000, 1000000000000),
        )

    def get_prices(self, symbols: list[str]) -> list[PriceData]:
        """Get prices for multiple assets."""
        # Clear cache at start of batch fetch
        self._price_cache.clear()

        results = []
        for symbol in symbols:
            data = self.get_price(symbol)
            if data:
                results.append(data)

        # Clear cache after batch
        self._price_cache.clear()
        return results


def create_consistent_mock_apis(variance_percent: float = 0.5) -> tuple[MockAPI, MockAPI]:
    """Create two mock APIs that return consistent prices.

    The second API will return prices within variance_percent of the first.

    Args:
        variance_percent: Maximum variance between the two APIs

    Returns:
        Tuple of (MockAPI for 'coingecko', MockAPI for 'coinmarketcap')
    """
    # First mock simulates CoinGecko
    mock_cg = MockAPI(source_suffix="", variance_percent=0.1, seed=42)
    # Override source name
    mock_cg._source_name_override = "coingecko"

    # Second mock simulates CoinMarketCap, staying consistent with first
    mock_cmc = MockAPI(
        source_suffix="",
        variance_percent=variance_percent,
        consistent_with=mock_cg,
        seed=43,
    )
    mock_cmc._source_name_override = "coinmarketcap"

    # Patch source_name property
    type(mock_cg).source_name = property(
        lambda self: getattr(self, "_source_name_override", f"mock{self._source_suffix}")
    )
    type(mock_cmc).source_name = property(
        lambda self: getattr(self, "_source_name_override", f"mock{self._source_suffix}")
    )

    return mock_cg, mock_cmc


def create_divergent_mock_apis(divergence_percent: float = 5.0) -> tuple[MockAPI, MockAPI]:
    """Create two mock APIs that return divergent prices.

    The second API will return prices that differ by at least divergence_percent.
    This is useful for testing validator rejection.

    Args:
        divergence_percent: Minimum divergence between the two APIs

    Returns:
        Tuple of (MockAPI for 'coingecko', MockAPI for 'coinmarketcap')
    """
    mock_cg, mock_cmc = create_consistent_mock_apis(divergence_percent)
    return mock_cg, mock_cmc
