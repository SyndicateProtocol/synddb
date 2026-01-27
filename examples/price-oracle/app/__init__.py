"""Price Oracle Example Application

This application demonstrates SyndDB custom validator rules by:
1. Fetching prices from multiple sources (CoinGecko, CoinMarketCap)
2. Writing both prices to the database (good logging practice)
3. Letting validators check price consistency (without needing API keys)
"""

__version__ = "0.1.0"
