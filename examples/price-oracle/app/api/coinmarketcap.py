"""CoinMarketCap API implementation

CoinMarketCap provides cryptocurrency price data.
Requires an API key (free tier available).
See: https://coinmarketcap.com/api/
"""

import logging
import time
from typing import Optional

import requests

from .base import PriceAPI, PriceData

logger = logging.getLogger(__name__)

# CoinMarketCap API endpoints
CMC_API_BASE = "https://pro-api.coinmarketcap.com/v1"
CMC_QUOTES_LATEST = f"{CMC_API_BASE}/cryptocurrency/quotes/latest"


# Map CoinGecko-style IDs to CMC symbols
ID_TO_SYMBOL = {
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


class CoinMarketCapAPI(PriceAPI):
    """CoinMarketCap price API.

    Requires an API key. Free tier allows 333 calls/day.
    """

    def __init__(self, api_key: str, timeout: int = 10):
        """Initialize CoinMarketCap API.

        Args:
            api_key: CMC API key (required)
            timeout: Request timeout in seconds
        """
        if not api_key:
            raise ValueError("CoinMarketCap API key is required")

        self._api_key = api_key
        self._timeout = timeout
        self._session = requests.Session()
        self._session.headers["X-CMC_PRO_API_KEY"] = api_key
        self._session.headers["Accept"] = "application/json"

    @property
    def source_name(self) -> str:
        return "coinmarketcap"

    def get_price(self, symbol: str) -> Optional[PriceData]:
        """Get price for a single asset."""
        prices = self.get_prices([symbol])
        return prices[0] if prices else None

    def get_prices(self, symbols: list[str]) -> list[PriceData]:
        """Get prices for multiple assets.

        CMC uses ticker symbols (BTC, ETH) rather than IDs,
        so we convert from CoinGecko-style IDs.
        """
        if not symbols:
            return []

        try:
            # Convert IDs to CMC symbols
            cmc_symbols = []
            symbol_mapping = {}  # CMC symbol -> original ID
            for sym in symbols:
                cmc_sym = ID_TO_SYMBOL.get(sym, sym.upper()[:4])
                cmc_symbols.append(cmc_sym)
                symbol_mapping[cmc_sym] = sym

            params = {
                "symbol": ",".join(cmc_symbols),
                "convert": "USD",
            }

            response = self._session.get(
                CMC_QUOTES_LATEST,
                params=params,
                timeout=self._timeout,
            )
            response.raise_for_status()
            data = response.json()

            if data.get("status", {}).get("error_code", 0) != 0:
                error_msg = data.get("status", {}).get("error_message", "Unknown error")
                logger.error(f"CMC API error: {error_msg}")
                return []

            timestamp = int(time.time())
            results = []

            for cmc_symbol, coin_data in data.get("data", {}).items():
                if isinstance(coin_data, list):
                    # CMC returns a list if multiple coins match
                    coin_data = coin_data[0]

                quote = coin_data.get("quote", {}).get("USD", {})
                price = quote.get("price")
                if price is None:
                    logger.warning(f"No USD price for {cmc_symbol}")
                    continue

                results.append(
                    PriceData(
                        asset=cmc_symbol,
                        source=self.source_name,
                        price=price,
                        timestamp=timestamp,
                        volume_24h=quote.get("volume_24h"),
                        market_cap=quote.get("market_cap"),
                    )
                )

            return results

        except requests.RequestException as e:
            logger.error(f"CoinMarketCap API error: {e}")
            return []
