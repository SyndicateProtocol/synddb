"""Integration tests for bridge message encoding and contract interaction."""

import asyncio
import subprocess
import time

import pytest

from price_oracle.bridge.client import BridgeClient


class TestCalldataEncoding:
    """Tests for calldata encoding matching Solidity expectations."""

    def test_encode_calldata_structure(self) -> None:
        """Test that the calldata encoding produces the expected structure."""
        client = BridgeClient("http://localhost:8080", "0x" + "00" * 32)

        # Test encoding
        asset = "bitcoin"
        price_scaled = 67196_645000000000000000  # ~$67,196.645
        timestamp = 1700000000

        calldata = client._encode_calldata(asset, price_scaled, timestamp)

        # Should start with 0x
        assert calldata.startswith("0x")

        # Should have function selector (4 bytes = 8 hex chars)
        selector = calldata[2:10]
        assert len(selector) == 8

        # Remaining data should be properly padded (multiple of 64 hex chars = 32 bytes)
        data = calldata[10:]
        # At minimum: offset (32) + price (32) + timestamp (32) + length (32) + asset data (32)
        assert len(data) >= 64 * 5

    def test_price_scaling(self) -> None:
        """Test that price scaling matches Solidity expectations."""
        # Python scales by 10^18
        price_usd = 67196.645
        price_scaled = int(price_usd * 10**18)

        # Should be approximately the expected value (within floating point tolerance)
        expected = 67196645000000000000000
        tolerance = 10**10  # Allow small floating point error
        assert abs(price_scaled - expected) < tolerance, f"Price {price_scaled} too far from {expected}"

        # In hex, should be representable in 32 bytes
        hex_price = hex(price_scaled)[2:]
        assert len(hex_price) <= 64  # 32 bytes = 64 hex chars

    def test_message_structure(self) -> None:
        """Test that the message structure matches validator expectations."""
        client = BridgeClient("http://localhost:8080", "0x" + "ab" * 32)
        client._nonce = 1

        # Build a message manually (same logic as push_price)
        asset = "ethereum"
        price = 3514.49
        timestamp = int(time.time())

        price_scaled = int(price * 10**18)
        nonce = client._get_nonce()
        now = int(time.time())
        calldata = client._encode_calldata(asset, price_scaled, timestamp)

        message = {
            "messageType": "updatePrice(string,uint256,uint256)",
            "calldata": calldata,
            "metadata": {
                "reason": "price_update",
                "asset": asset,
                "price_scaled": str(price_scaled),
                "timestamp": timestamp,
                "source": "price-oracle",
            },
            "nonce": nonce,
            "timestamp": now,
            "domain": client.domain,
        }

        # Verify message structure
        assert message["messageType"] == "updatePrice(string,uint256,uint256)"
        assert message["calldata"].startswith("0x")
        assert message["nonce"] == 1
        assert message["domain"] == "0x" + "ab" * 32
        assert "metadata" in message
        assert message["metadata"]["asset"] == "ethereum"
        assert message["metadata"]["source"] == "price-oracle"


class TestBridgeClientNonceManagement:
    """Tests for nonce management in bridge client."""

    def test_nonce_increments(self) -> None:
        """Test that nonce increments correctly."""
        client = BridgeClient("http://localhost:8080", "0x" + "00" * 32)

        assert client._get_nonce() == 1
        assert client._get_nonce() == 2
        assert client._get_nonce() == 3

    def test_nonce_starts_at_one(self) -> None:
        """Test that nonce starts at 1."""
        client = BridgeClient("http://localhost:8080", "0x" + "00" * 32)
        assert client._nonce == 1


class TestPriceOracleEndToEnd:
    """End-to-end tests for the price oracle workflow."""

    @pytest.mark.asyncio
    async def test_mock_price_consistency(self) -> None:
        """Test that mock prices are consistent within tolerance."""
        from price_oracle.sources import MockMode, MockSource

        source1 = MockSource("mock1", MockMode.CONSISTENT, seed=42)
        source2 = MockSource("mock2", MockMode.CONSISTENT, seed=43)

        prices1 = await source1.fetch_prices(["bitcoin"])
        prices2 = await source2.fetch_prices(["bitcoin"])

        # Calculate difference in basis points
        avg_price = (prices1[0].price + prices2[0].price) / 2
        diff = abs(prices1[0].price - prices2[0].price)
        diff_bps = int((diff / avg_price) * 10000)

        # Should be within 100 bps (1%)
        assert diff_bps < 100, f"Price difference {diff_bps} bps exceeds threshold"

    @pytest.mark.asyncio
    async def test_divergent_price_detection(self) -> None:
        """Test that divergent prices are detected."""
        from price_oracle.comparison import PriceDivergenceError, compare_prices
        from price_oracle.models import PriceRecord
        from price_oracle.sources import MockMode, MockSource

        source1 = MockSource("mock1", MockMode.CONSISTENT, seed=42)
        source2 = MockSource("mock2", MockMode.DIVERGENT, seed=43)

        prices1 = await source1.fetch_prices(["bitcoin"])
        prices2 = await source2.fetch_prices(["bitcoin"])

        now = int(time.time())
        records = [
            PriceRecord(
                asset="bitcoin",
                source="mock1",
                price=prices1[0].price,
                timestamp=now,
            ),
            PriceRecord(
                asset="bitcoin",
                source="mock2",
                price=prices2[0].price,
                timestamp=now,
            ),
        ]

        # Should raise PriceDivergenceError
        with pytest.raises(PriceDivergenceError) as exc_info:
            compare_prices(records, max_diff_bps=100)

        assert exc_info.value.actual_bps > 100

    @pytest.mark.asyncio
    async def test_full_workflow_mock(self) -> None:
        """Test full workflow with mock sources."""
        from price_oracle.comparison import compare_prices
        from price_oracle.db import PriceStore
        from price_oracle.models import PriceRecord
        from price_oracle.sources import MockMode, MockSource

        # Initialize store
        store = PriceStore(":memory:")
        store.init_schema()

        # Create sources
        source1 = MockSource("mock1", MockMode.CONSISTENT, seed=42)
        source2 = MockSource("mock2", MockMode.CONSISTENT, seed=43)

        # Get assets
        assets = store.get_active_assets()
        asset_symbols = [a.symbol for a in assets]

        now = int(time.time())

        # Fetch and store prices
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

        # Compare prices for each asset
        for asset in assets:
            records = store.get_latest_price_per_source(asset.symbol)
            assert len(records) == 2

            # Should pass validation
            result = compare_prices(records, max_diff_bps=100)
            assert result.difference_bps < 100

            # Price is ready to be pushed to Bridge
            avg_price_scaled = int(result.avg_price * 10**18)
            assert avg_price_scaled > 0

            # Verify it can be encoded for the contract
            client = BridgeClient("http://localhost:8080", "0x" + "00" * 32)
            calldata = client._encode_calldata(asset.symbol, avg_price_scaled, now)
            assert calldata.startswith("0x")
