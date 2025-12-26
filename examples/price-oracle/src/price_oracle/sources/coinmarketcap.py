"""CoinMarketCap price source."""

import logging
from typing import Optional

import httpx

from ..models import FetchedPrice
from .base import PriceSource

logger = logging.getLogger(__name__)

CMC_API_URL = "https://pro-api.coinmarketcap.com/v1/cryptocurrency/quotes/latest"

SYMBOL_MAP = {
    "bitcoin": "BTC",
    "ethereum": "ETH",
    "solana": "SOL",
    "cardano": "ADA",
    "polkadot": "DOT",
    "ripple": "XRP",
    "dogecoin": "DOGE",
    "avalanche": "AVAX",
    "chainlink": "LINK",
    "polygon": "MATIC",
}


class CoinMarketCapSource(PriceSource):
    """CoinMarketCap API price source."""

    def __init__(self, api_key: str):
        self.api_key = api_key
        self._client = httpx.AsyncClient(timeout=30.0)
        self._symbol_map = SYMBOL_MAP.copy()
        self._reverse_map = {v: k for k, v in SYMBOL_MAP.items()}

    @property
    def name(self) -> str:
        return "coinmarketcap"

    def _to_cmc_symbol(self, asset: str) -> Optional[str]:
        """Convert our asset name to CMC symbol."""
        return self._symbol_map.get(asset)

    async def fetch_prices(self, assets: list[str]) -> list[FetchedPrice]:
        """Fetch prices from CoinMarketCap API."""
        if not assets:
            return []

        cmc_symbols = [
            self._to_cmc_symbol(a) for a in assets if self._to_cmc_symbol(a)
        ]
        if not cmc_symbols:
            return []

        symbols_param = ",".join(cmc_symbols)
        params = {"symbol": symbols_param, "convert": "USD"}
        headers = {
            "X-CMC_PRO_API_KEY": self.api_key,
            "Accept": "application/json",
        }

        try:
            response = await self._client.get(CMC_API_URL, params=params, headers=headers)

            if response.status_code == 429:
                logger.warning("CoinMarketCap rate limited")
                return []

            response.raise_for_status()
            data = response.json()

            results = []
            for symbol, asset_data in data.get("data", {}).items():
                our_symbol = self._reverse_map.get(symbol)
                if our_symbol and "quote" in asset_data and "USD" in asset_data["quote"]:
                    quote = asset_data["quote"]["USD"]
                    results.append(
                        FetchedPrice(
                            asset=our_symbol,
                            price=quote.get("price", 0),
                            volume_24h=quote.get("volume_24h"),
                            market_cap=quote.get("market_cap"),
                        )
                    )
            return results

        except httpx.HTTPError as e:
            logger.error(f"CoinMarketCap API error: {e}")
            return []
