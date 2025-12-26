"""Tests for the price oracle."""

import pytest

from price_oracle.comparison import (
    InsufficientSourcesError,
    PriceDivergenceError,
    compare_prices,
)
from price_oracle.db import PriceStore
from price_oracle.models import PriceRecord
from price_oracle.sources import MockMode, MockSource


class TestPriceStore:
    """Tests for the PriceStore."""

    def test_init_schema(self) -> None:
        store = PriceStore(":memory:")
        store.init_schema()
        assets = store.get_active_assets()
        assert len(assets) == 2
        symbols = {a.symbol for a in assets}
        assert "bitcoin" in symbols
        assert "ethereum" in symbols

    def test_add_asset(self) -> None:
        store = PriceStore(":memory:")
        store.init_schema()
        store.add_asset("solana", "SOL")
        assets = store.get_active_assets()
        assert len(assets) == 3
        symbols = {a.symbol for a in assets}
        assert "solana" in symbols

    def test_insert_and_get_prices(self) -> None:
        store = PriceStore(":memory:")
        store.init_schema()

        record = PriceRecord(
            asset="bitcoin",
            source="test",
            price=67000.0,
            volume_24h=1000000.0,
            market_cap=1000000000.0,
            timestamp=1234567890,
        )
        store.insert_price(record)

        prices = store.get_prices(asset="bitcoin")
        assert len(prices) == 1
        assert prices[0].price == 67000.0
        assert prices[0].source == "test"


class TestMockSource:
    """Tests for the MockSource."""

    @pytest.mark.asyncio
    async def test_consistent_mode(self) -> None:
        source1 = MockSource("mock1", MockMode.CONSISTENT, seed=42)
        source2 = MockSource("mock2", MockMode.CONSISTENT, seed=43)

        prices1 = await source1.fetch_prices(["bitcoin"])
        prices2 = await source2.fetch_prices(["bitcoin"])

        assert len(prices1) == 1
        assert len(prices2) == 1

        diff = abs(prices1[0].price - prices2[0].price)
        avg = (prices1[0].price + prices2[0].price) / 2
        diff_pct = diff / avg

        assert diff_pct < 0.01

    @pytest.mark.asyncio
    async def test_divergent_mode(self) -> None:
        source1 = MockSource("mock1", MockMode.CONSISTENT, seed=42)
        source2 = MockSource("mock2", MockMode.DIVERGENT, seed=43)

        prices1 = await source1.fetch_prices(["bitcoin"])
        prices2 = await source2.fetch_prices(["bitcoin"])

        assert len(prices1) == 1
        assert len(prices2) == 1


class TestComparison:
    """Tests for price comparison."""

    def test_compare_prices_within_tolerance(self) -> None:
        prices = [
            PriceRecord(
                asset="bitcoin",
                source="mock1",
                price=67000.0,
                timestamp=1234567890,
            ),
            PriceRecord(
                asset="bitcoin",
                source="mock2",
                price=67100.0,
                timestamp=1234567890,
            ),
        ]
        result = compare_prices(prices, max_diff_bps=100)
        assert result.asset == "bitcoin"
        assert result.difference_bps < 100
        assert len(result.sources) == 2

    def test_compare_prices_exceeds_tolerance(self) -> None:
        prices = [
            PriceRecord(
                asset="bitcoin",
                source="mock1",
                price=67000.0,
                timestamp=1234567890,
            ),
            PriceRecord(
                asset="bitcoin",
                source="mock2",
                price=70000.0,
                timestamp=1234567890,
            ),
        ]
        with pytest.raises(PriceDivergenceError):
            compare_prices(prices, max_diff_bps=100)

    def test_compare_prices_insufficient_sources(self) -> None:
        prices = [
            PriceRecord(
                asset="bitcoin",
                source="mock1",
                price=67000.0,
                timestamp=1234567890,
            ),
        ]
        with pytest.raises(InsufficientSourcesError):
            compare_prices(prices, max_diff_bps=100)


class TestIntegration:
    """Integration tests."""

    @pytest.mark.asyncio
    async def test_full_workflow(self) -> None:
        store = PriceStore(":memory:")
        store.init_schema()

        source1 = MockSource("mock1", MockMode.CONSISTENT, seed=42)
        source2 = MockSource("mock2", MockMode.CONSISTENT, seed=43)

        assets = store.get_active_assets()
        asset_symbols = [a.symbol for a in assets]

        import time

        now = int(time.time())

        for source in [source1, source2]:
            prices = await source.fetch_prices(asset_symbols)
            for price in prices:
                record = PriceRecord(
                    asset=price.asset,
                    source=source.name,
                    price=price.price,
                    volume_24h=price.volume_24h,
                    market_cap=price.market_cap,
                    timestamp=now,
                )
                store.insert_price(record)

        for asset in assets:
            records = store.get_latest_price_per_source(asset.symbol)
            assert len(records) == 2

            result = compare_prices(records, max_diff_bps=100)
            assert result.difference_bps < 100

    @pytest.mark.asyncio
    async def test_divergent_workflow_fails(self) -> None:
        store = PriceStore(":memory:")
        store.init_schema()

        source1 = MockSource("mock1", MockMode.CONSISTENT, seed=42)
        source2 = MockSource("mock2", MockMode.DIVERGENT, seed=43)

        import time

        now = int(time.time())

        for source in [source1, source2]:
            prices = await source.fetch_prices(["bitcoin"])
            for price in prices:
                record = PriceRecord(
                    asset=price.asset,
                    source=source.name,
                    price=price.price,
                    timestamp=now,
                )
                store.insert_price(record)

        records = store.get_latest_price_per_source("bitcoin")
        assert len(records) == 2

        with pytest.raises(PriceDivergenceError):
            compare_prices(records, max_diff_bps=100)
