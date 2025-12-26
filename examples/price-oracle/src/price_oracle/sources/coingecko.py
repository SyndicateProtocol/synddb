"""CoinGecko price source."""

import logging
from typing import Optional

import httpx

from ..models import FetchedPrice
from .base import PriceSource

logger = logging.getLogger(__name__)

COINGECKO_API_URL = "https://api.coingecko.com/api/v3/simple/price"


class CoinGeckoSource(PriceSource):
    """CoinGecko API price source."""

    def __init__(self, api_key: Optional[str] = None):
        self.api_key = api_key
        self._client = httpx.AsyncClient(timeout=30.0)

    @property
    def name(self) -> str:
        return "coingecko"

    async def fetch_prices(self, assets: list[str]) -> list[FetchedPrice]:
        """Fetch prices from CoinGecko API."""
        if not assets:
            return []

        ids = ",".join(assets)
        params = {
            "ids": ids,
            "vs_currencies": "usd",
            "include_market_cap": "true",
            "include_24hr_vol": "true",
        }

        headers = {}
        if self.api_key:
            headers["x-cg-demo-api-key"] = self.api_key

        try:
            response = await self._client.get(
                COINGECKO_API_URL, params=params, headers=headers
            )

            if response.status_code == 429:
                logger.warning("CoinGecko rate limited")
                return []

            response.raise_for_status()
            data = response.json()

            results = []
            for asset, prices in data.items():
                if "usd" in prices:
                    results.append(
                        FetchedPrice(
                            asset=asset,
                            price=prices["usd"],
                            volume_24h=prices.get("usd_24h_vol"),
                            market_cap=prices.get("usd_market_cap"),
                        )
                    )
            return results

        except httpx.HTTPError as e:
            logger.error(f"CoinGecko API error: {e}")
            return []
