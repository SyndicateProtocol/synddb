"""CoinGecko API implementation

CoinGecko provides free cryptocurrency price data with rate limits.
See: https://www.coingecko.com/en/api
"""

import logging
import time
from typing import Optional

import requests

from .base import PriceAPI, PriceData

logger = logging.getLogger(__name__)

# CoinGecko API endpoints
COINGECKO_API_BASE = "https://api.coingecko.com/api/v3"
COINGECKO_SIMPLE_PRICE = f"{COINGECKO_API_BASE}/simple/price"


class CoinGeckoAPI(PriceAPI):
    """CoinGecko price API.

    Free tier allows 10-50 calls/minute depending on endpoint.
    Pro API key can be provided for higher rate limits.
    """

    def __init__(self, api_key: Optional[str] = None, timeout: int = 10):
        """Initialize CoinGecko API.

        Args:
            api_key: Optional Pro API key for higher rate limits
            timeout: Request timeout in seconds
        """
        self._api_key = api_key
        self._timeout = timeout
        self._session = requests.Session()

        if api_key:
            self._session.headers["x-cg-pro-api-key"] = api_key

    @property
    def source_name(self) -> str:
        return "coingecko"

    def get_price(self, symbol: str) -> Optional[PriceData]:
        """Get price for a single asset."""
        prices = self.get_prices([symbol])
        return prices[0] if prices else None

    def get_prices(self, symbols: list[str]) -> list[PriceData]:
        """Get prices for multiple assets.

        CoinGecko supports batch queries which is more efficient.
        """
        if not symbols:
            return []

        try:
            params = {
                "ids": ",".join(symbols),
                "vs_currencies": "usd",
                "include_24hr_vol": "true",
                "include_market_cap": "true",
            }

            response = self._session.get(
                COINGECKO_SIMPLE_PRICE,
                params=params,
                timeout=self._timeout,
            )
            response.raise_for_status()
            data = response.json()

            timestamp = int(time.time())
            results = []

            for symbol in symbols:
                if symbol not in data:
                    logger.warning(f"No data for {symbol} from CoinGecko")
                    continue

                coin_data = data[symbol]
                price = coin_data.get("usd")
                if price is None:
                    logger.warning(f"No USD price for {symbol}")
                    continue

                # Map symbol to display name (e.g., 'bitcoin' -> 'BTC')
                display_name = self._symbol_to_display(symbol)

                results.append(
                    PriceData(
                        asset=display_name,
                        source=self.source_name,
                        price=price,
                        timestamp=timestamp,
                        volume_24h=coin_data.get("usd_24h_vol"),
                        market_cap=coin_data.get("usd_market_cap"),
                    )
                )

            return results

        except requests.RequestException as e:
            logger.error(f"CoinGecko API error: {e}")
            return []

    def _symbol_to_display(self, symbol: str) -> str:
        """Convert API symbol to display name."""
        mapping = {
            "bitcoin": "BTC",
            "ethereum": "ETH",
            "solana": "SOL",
            "cardano": "ADA",
            "polkadot": "DOT",
            "ripple": "XRP",
            "dogecoin": "DOGE",
            "avalanche-2": "AVAX",
            "chainlink": "LINK",
            "polygon": "MATIC",
        }
        return mapping.get(symbol, symbol.upper()[:4])
