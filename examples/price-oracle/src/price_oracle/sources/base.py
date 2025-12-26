"""Base price source interface."""

from abc import ABC, abstractmethod

from ..models import FetchedPrice


class PriceSource(ABC):
    """Abstract base class for price data sources."""

    @property
    @abstractmethod
    def name(self) -> str:
        """Return the source name."""
        ...

    @abstractmethod
    async def fetch_prices(self, assets: list[str]) -> list[FetchedPrice]:
        """Fetch prices for the given assets."""
        ...
