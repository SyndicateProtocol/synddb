"""Mock price source for testing."""

from enum import Enum

from ..models import FetchedPrice
from .base import PriceSource


class MockMode(str, Enum):
    """Mock source behavior mode."""

    CONSISTENT = "consistent"
    DIVERGENT = "divergent"


DEFAULT_BASE_PRICES = {
    "bitcoin": 67000.0,
    "ethereum": 3500.0,
    "solana": 150.0,
    "cardano": 0.45,
    "polkadot": 7.5,
    "ripple": 0.55,
    "dogecoin": 0.12,
    "avalanche": 35.0,
    "chainlink": 15.0,
    "polygon": 0.85,
}


class MockSource(PriceSource):
    """Mock price source with configurable behavior."""

    def __init__(
        self,
        name: str = "mock",
        mode: MockMode = MockMode.CONSISTENT,
        seed: int = 42,
    ):
        self._name = name
        self.mode = mode
        self.seed = seed
        self.base_prices = DEFAULT_BASE_PRICES.copy()

        self.variance_pct = 0.005 if mode == MockMode.CONSISTENT else 0.05

    @property
    def name(self) -> str:
        return self._name

    def _generate_price(self, base_price: float, asset: str) -> float:
        """Generate a deterministic price with variance."""
        hash_val = (self.seed + len(asset) + sum(ord(c) for c in asset)) % 1000
        factor = hash_val / 1000.0

        variance = self.variance_pct * (2.0 * factor - 1.0)
        return base_price * (1.0 + variance)

    async def fetch_prices(self, assets: list[str]) -> list[FetchedPrice]:
        """Fetch mock prices for the given assets."""
        results = []
        for asset in assets:
            if asset in self.base_prices:
                base = self.base_prices[asset]
                price = self._generate_price(base, asset)
                results.append(
                    FetchedPrice(
                        asset=asset,
                        price=price,
                        volume_24h=1_000_000.0,
                        market_cap=base * 1_000_000_000.0,
                    )
                )
        return results
