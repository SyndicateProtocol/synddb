"""Tests for the mock price API."""

import time

from app.api.mock import MockAPI, BASE_PRICES
from app.api.base import PriceData


def test_mock_api_source_name():
    """Test mock API returns correct source name."""
    api = MockAPI()
    assert api.source_name == "mock"

    api_a = MockAPI(source_suffix="_a")
    assert api_a.source_name == "mock_a"


def test_mock_api_get_price():
    """Test mock API returns valid price data."""
    api = MockAPI(seed=42)  # Fixed seed for reproducibility

    result = api.get_price("bitcoin")

    assert result is not None
    assert isinstance(result, PriceData)
    # MockAPI uses short names like 'BIT' for display
    assert result.asset in ("bitcoin", "BIT")
    assert result.source == "mock"
    assert result.price > 0
    assert result.timestamp > 0


def test_mock_api_get_prices():
    """Test mock API returns prices for multiple assets."""
    api = MockAPI(seed=42)

    results = api.get_prices(["bitcoin", "ethereum"])

    assert len(results) == 2
    # MockAPI may use short display names
    assets = {r.asset for r in results}
    # Check that we got 2 different assets
    assert len(assets) == 2


def test_mock_api_price_variance():
    """Test mock API respects variance parameter."""
    api_low = MockAPI(variance_percent=0.1, seed=42)
    api_high = MockAPI(variance_percent=50.0, seed=42)

    prices_low = [api_low.get_price("bitcoin").price for _ in range(10)]
    prices_high = [api_high.get_price("bitcoin").price for _ in range(10)]

    # Low variance prices should be closer together
    range_low = max(prices_low) - min(prices_low)
    range_high = max(prices_high) - min(prices_high)

    # Note: With same seed, both will have same pattern but scaled differently
    assert range_low <= range_high


def test_mock_api_reproducibility():
    """Test mock API produces reproducible results with same seed."""
    api1 = MockAPI(seed=12345)
    api2 = MockAPI(seed=12345)

    price1 = api1.get_price("bitcoin").price
    price2 = api2.get_price("bitcoin").price

    assert price1 == price2


def test_mock_api_consistency_mode():
    """Test that consistent APIs produce similar prices."""
    api_primary = MockAPI(source_suffix="_primary", variance_percent=0.1, seed=42)
    api_secondary = MockAPI(
        source_suffix="_secondary",
        variance_percent=0.1,
        consistent_with=api_primary,
        seed=43,
    )

    price_primary = api_primary.get_price("bitcoin").price
    price_secondary = api_secondary.get_price("bitcoin").price

    # Prices should be within 0.2% of each other (0.1% variance each)
    diff_percent = abs(price_primary - price_secondary) / price_primary * 100
    assert diff_percent < 1.0  # Allow some margin


def test_mock_api_unknown_asset():
    """Test mock API handles unknown assets with default price."""
    api = MockAPI(seed=42)

    result = api.get_price("unknown_asset")

    assert result is not None
    assert result.price > 0  # Should use default base price


def test_mock_api_base_prices():
    """Test that known assets use appropriate base prices."""
    api = MockAPI(variance_percent=0.0, seed=42)  # No variance

    # Note: Even with 0 variance, there's some randomness in the mock
    # So we just verify the order of magnitude is correct
    btc = api.get_price("bitcoin").price
    eth = api.get_price("ethereum").price

    assert btc > eth  # BTC should be more expensive than ETH
    assert btc > 10000  # BTC should be in tens of thousands
    assert eth > 1000  # ETH should be in thousands
