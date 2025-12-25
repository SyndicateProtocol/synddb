"""Base class for price APIs

All price API implementations must inherit from PriceAPI.
"""

from abc import ABC, abstractmethod
from dataclasses import dataclass
from typing import Optional


@dataclass
class PriceData:
    """Price data returned from an API.

    Attributes:
        asset: Asset symbol (e.g., 'BTC', 'ETH')
        source: Source identifier (e.g., 'coingecko', 'coinmarketcap')
        price: Current price in USD
        timestamp: Unix timestamp when price was fetched
        volume_24h: Optional 24h trading volume
        market_cap: Optional market capitalization
    """

    asset: str
    source: str
    price: float
    timestamp: int
    volume_24h: Optional[float] = None
    market_cap: Optional[float] = None


class PriceAPI(ABC):
    """Abstract base class for price APIs.

    All price API implementations must implement:
    - source_name: Returns the identifier for this source
    - get_price: Fetches price for a single asset
    - get_prices: Fetches prices for multiple assets
    """

    @property
    @abstractmethod
    def source_name(self) -> str:
        """Return the source identifier (e.g., 'coingecko')."""
        pass

    @abstractmethod
    def get_price(self, symbol: str) -> Optional[PriceData]:
        """Fetch price for a single asset.

        Args:
            symbol: Asset symbol in API format (e.g., 'bitcoin', 'ethereum')

        Returns:
            PriceData if successful, None if failed
        """
        pass

    @abstractmethod
    def get_prices(self, symbols: list[str]) -> list[PriceData]:
        """Fetch prices for multiple assets.

        Args:
            symbols: List of asset symbols in API format

        Returns:
            List of PriceData for successful fetches
        """
        pass
