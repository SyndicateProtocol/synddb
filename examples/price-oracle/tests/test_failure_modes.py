"""Tests for Bridge client failure handling.

These tests demonstrate the failure handling patterns required for message-passing:
- Retry logic with exponential backoff
- Error classification (retryable vs non-retryable)
- Status polling and timeout handling
- Connection error handling

In contrast, SQLite transactions are atomic and don't require these patterns.
"""

import asyncio
from unittest.mock import AsyncMock, MagicMock, patch

import httpx
import pytest

from price_oracle.bridge import (
    BridgeClient,
    MessageStatus,
    PushResult,
    RETRYABLE_ERRORS,
    is_retryable_error,
    stage_to_status,
)


class TestErrorClassification:
    """Tests for error code classification."""

    def test_retryable_errors(self) -> None:
        """Verify retryable errors are correctly classified."""
        for error_code in RETRYABLE_ERRORS:
            assert is_retryable_error(error_code) is True

    def test_non_retryable_errors(self) -> None:
        """Verify non-retryable errors are correctly classified."""
        non_retryable = [
            "REPLAY_DETECTED",
            "INVALID_NONCE",
            "TIMESTAMP_EXPIRED",
            "APP_NOT_AUTHORIZED",
            "INVARIANT_VIOLATED",
            "SCHEMA_VALIDATION_FAILED",
        ]
        for error_code in non_retryable:
            assert is_retryable_error(error_code) is False

    def test_none_error(self) -> None:
        """Verify None is not retryable."""
        assert is_retryable_error(None) is False


class TestStageMapping:
    """Tests for stage to status mapping."""

    def test_all_stages_mapped(self) -> None:
        """Verify all stages have status strings."""
        for stage in range(9):
            status = stage_to_status(stage)
            assert status is not None
            assert not status.startswith("unknown")

    def test_unknown_stage(self) -> None:
        """Verify unknown stages return a reasonable default."""
        status = stage_to_status(99)
        assert "unknown" in status


class TestMessageStatus:
    """Tests for MessageStatus model."""

    def test_terminal_states(self) -> None:
        """Verify terminal state detection."""
        # Completed is terminal
        completed = MessageStatus(
            message_id="0x123",
            stage=6,
            status="completed",
            executed=True,
        )
        assert completed.is_terminal is True
        assert completed.is_success is True

        # Failed is terminal
        failed = MessageStatus(
            message_id="0x123",
            stage=7,
            status="failed",
            executed=False,
        )
        assert failed.is_terminal is True
        assert failed.is_success is False

        # Expired is terminal
        expired = MessageStatus(
            message_id="0x123",
            stage=8,
            status="expired",
            executed=False,
        )
        assert expired.is_terminal is True
        assert expired.is_success is False

    def test_non_terminal_states(self) -> None:
        """Verify non-terminal state detection."""
        for stage in [0, 1, 2, 3, 4, 5]:
            status = MessageStatus(
                message_id="0x123",
                stage=stage,
                status=stage_to_status(stage),
                executed=False,
            )
            assert status.is_terminal is False
            assert status.is_success is False


class TestPushResult:
    """Tests for PushResult model."""

    def test_success_result(self) -> None:
        """Verify successful push result."""
        result = PushResult(
            success=True,
            message_id="0x123",
            signature="0xabc",
            is_retryable=False,
            attempts=1,
        )
        assert result.success is True
        assert result.message_id == "0x123"

    def test_retryable_failure(self) -> None:
        """Verify retryable failure result."""
        result = PushResult(
            success=False,
            error_code="BRIDGE_CONNECTION_FAILED",
            error_message="Connection refused",
            is_retryable=True,
            attempts=2,
        )
        assert result.success is False
        assert result.is_retryable is True

    def test_non_retryable_failure(self) -> None:
        """Verify non-retryable failure result."""
        result = PushResult(
            success=False,
            error_code="INVALID_NONCE",
            error_message="Nonce already used",
            is_retryable=False,
            attempts=1,
        )
        assert result.success is False
        assert result.is_retryable is False


class TestBridgeClientRetry:
    """Tests for BridgeClient retry logic."""

    @pytest.mark.asyncio
    async def test_successful_push_no_retry(self) -> None:
        """Verify successful push doesn't retry."""
        client = BridgeClient(
            validator_url="http://localhost:8080",
            domain="0x" + "ab" * 32,
            max_retries=3,
        )

        with patch.object(client, "_submit_message_detailed") as mock_submit:
            mock_submit.return_value = PushResult(
                success=True,
                message_id="0x123",
                signature="0xabc",
                is_retryable=False,
                attempts=1,
            )

            result = await client.push_price_with_retry("bitcoin", 67000.0)

            assert result.success is True
            assert result.attempts == 1
            assert mock_submit.call_count == 1

        await client.close()

    @pytest.mark.asyncio
    async def test_retry_on_transient_error(self) -> None:
        """Verify retry on transient errors with backoff."""
        client = BridgeClient(
            validator_url="http://localhost:8080",
            domain="0x" + "ab" * 32,
            max_retries=3,
            retry_delay=0.01,  # Fast for testing
        )

        call_count = 0

        async def mock_submit(message):
            nonlocal call_count
            call_count += 1
            if call_count < 3:
                return PushResult(
                    success=False,
                    error_code="BRIDGE_CONNECTION_FAILED",
                    error_message="Connection refused",
                    is_retryable=True,
                    attempts=1,
                )
            return PushResult(
                success=True,
                message_id="0x123",
                signature="0xabc",
                is_retryable=False,
                attempts=1,
            )

        with patch.object(client, "_submit_message_detailed", side_effect=mock_submit):
            result = await client.push_price_with_retry("bitcoin", 67000.0)

            assert result.success is True
            assert result.attempts == 3
            assert call_count == 3

        await client.close()

    @pytest.mark.asyncio
    async def test_no_retry_on_non_retryable_error(self) -> None:
        """Verify no retry on non-retryable errors."""
        client = BridgeClient(
            validator_url="http://localhost:8080",
            domain="0x" + "ab" * 32,
            max_retries=3,
        )

        with patch.object(client, "_submit_message_detailed") as mock_submit:
            mock_submit.return_value = PushResult(
                success=False,
                error_code="INVALID_NONCE",
                error_message="Nonce already used",
                is_retryable=False,
                attempts=1,
            )

            result = await client.push_price_with_retry("bitcoin", 67000.0)

            assert result.success is False
            assert result.error_code == "INVALID_NONCE"
            assert result.attempts == 1
            assert mock_submit.call_count == 1

        await client.close()

    @pytest.mark.asyncio
    async def test_max_retries_exceeded(self) -> None:
        """Verify max retries limit is respected."""
        client = BridgeClient(
            validator_url="http://localhost:8080",
            domain="0x" + "ab" * 32,
            max_retries=3,
            retry_delay=0.01,
        )

        with patch.object(client, "_submit_message_detailed") as mock_submit:
            mock_submit.return_value = PushResult(
                success=False,
                error_code="BRIDGE_CONNECTION_FAILED",
                error_message="Connection refused",
                is_retryable=True,
                attempts=1,
            )

            result = await client.push_price_with_retry("bitcoin", 67000.0)

            assert result.success is False
            assert result.error_code == "MAX_RETRIES_EXCEEDED"
            assert result.attempts == 3
            assert mock_submit.call_count == 3

        await client.close()


class TestBridgeClientStatusPolling:
    """Tests for BridgeClient status polling."""

    @pytest.mark.asyncio
    async def test_wait_for_completion_success(self) -> None:
        """Verify wait_for_completion returns on success."""
        client = BridgeClient(
            validator_url="http://localhost:8080",
            domain="0x" + "ab" * 32,
        )

        call_count = 0

        async def mock_status(message_id):
            nonlocal call_count
            call_count += 1
            if call_count < 3:
                return MessageStatus(
                    message_id=message_id,
                    stage=1,  # pending
                    status="pending",
                    executed=False,
                )
            return MessageStatus(
                message_id=message_id,
                stage=6,  # completed
                status="completed",
                executed=True,
            )

        with patch.object(client, "get_message_status", side_effect=mock_status):
            status = await client.wait_for_completion(
                "0x123",
                timeout=5.0,
                poll_interval=0.01,
            )

            assert status.is_terminal is True
            assert status.is_success is True
            assert status.stage == 6

        await client.close()

    @pytest.mark.asyncio
    async def test_wait_for_completion_failure(self) -> None:
        """Verify wait_for_completion returns on failure."""
        client = BridgeClient(
            validator_url="http://localhost:8080",
            domain="0x" + "ab" * 32,
        )

        async def mock_status(message_id):
            return MessageStatus(
                message_id=message_id,
                stage=7,  # failed
                status="failed",
                executed=False,
            )

        with patch.object(client, "get_message_status", side_effect=mock_status):
            status = await client.wait_for_completion(
                "0x123",
                timeout=5.0,
                poll_interval=0.01,
            )

            assert status.is_terminal is True
            assert status.is_success is False
            assert status.stage == 7

        await client.close()

    @pytest.mark.asyncio
    async def test_wait_for_completion_timeout(self) -> None:
        """Verify wait_for_completion respects timeout."""
        client = BridgeClient(
            validator_url="http://localhost:8080",
            domain="0x" + "ab" * 32,
        )

        async def mock_status(message_id):
            return MessageStatus(
                message_id=message_id,
                stage=1,  # pending - never transitions
                status="pending",
                executed=False,
            )

        with patch.object(client, "get_message_status", side_effect=mock_status):
            status = await client.wait_for_completion(
                "0x123",
                timeout=0.05,
                poll_interval=0.01,
            )

            # Should timeout without reaching terminal state
            assert status.is_terminal is False
            assert status.stage == 1

        await client.close()


class TestBridgeClientConnectionErrors:
    """Tests for BridgeClient connection error handling."""

    @pytest.mark.asyncio
    async def test_connection_error_is_retryable(self) -> None:
        """Verify connection errors are classified as retryable."""
        client = BridgeClient(
            validator_url="http://localhost:8080",
            domain="0x" + "ab" * 32,
        )

        with patch.object(client._client, "post") as mock_post:
            mock_post.side_effect = httpx.ConnectError("Connection refused")

            result = await client._submit_message_detailed({})

            assert result.success is False
            assert result.error_code == "CONNECTION_ERROR"
            assert result.is_retryable is True

        await client.close()

    @pytest.mark.asyncio
    async def test_timeout_error_is_retryable(self) -> None:
        """Verify timeout errors are classified as retryable."""
        client = BridgeClient(
            validator_url="http://localhost:8080",
            domain="0x" + "ab" * 32,
        )

        with patch.object(client._client, "post") as mock_post:
            mock_post.side_effect = httpx.TimeoutException("Request timed out")

            result = await client._submit_message_detailed({})

            assert result.success is False
            assert result.error_code == "TIMEOUT"
            assert result.is_retryable is True

        await client.close()


class TestComplexQueriesStore:
    """Tests for complex query store methods."""

    def test_price_stats_calculation(self) -> None:
        """Verify price stats are calculated correctly."""
        import time

        from price_oracle.db import PriceStore
        from price_oracle.models import PriceRecord

        store = PriceStore(":memory:")
        store.init_schema()

        # Insert some test prices
        now = int(time.time())
        prices = [100.0, 102.0, 98.0, 101.0, 99.0]

        for i, price in enumerate(prices):
            record = PriceRecord(
                asset="bitcoin",
                source=f"source{i}",
                price=price,
                timestamp=now - i,
            )
            store.insert_price(record)

        # Get stats for last hour
        stats = store.get_price_stats("bitcoin", 3600)

        assert stats is not None
        assert stats.count == 5
        assert stats.avg_price == pytest.approx(100.0, rel=0.01)
        assert stats.min_price == 98.0
        assert stats.max_price == 102.0
        assert stats.stddev > 0
        assert stats.volatility_pct > 0

    def test_ohlc_calculation(self) -> None:
        """Verify OHLC candles are calculated correctly."""
        import time

        from price_oracle.db import PriceStore
        from price_oracle.models import PriceRecord

        store = PriceStore(":memory:")
        store.init_schema()

        # Insert prices within a single 1-minute bucket
        now = int(time.time())
        bucket_start = (now // 60) * 60

        # Open, then low, then high, then close
        test_prices = [
            (bucket_start, 100.0),  # Open
            (bucket_start + 10, 95.0),  # Low
            (bucket_start + 20, 110.0),  # High
            (bucket_start + 30, 105.0),  # Close
        ]

        for ts, price in test_prices:
            record = PriceRecord(
                asset="bitcoin",
                source="test",
                price=price,
                timestamp=ts,
            )
            store.insert_price(record)

        # Get 1-minute OHLC
        candles = store.get_ohlc("bitcoin", "1m", limit=10)

        assert len(candles) >= 1
        candle = candles[0]
        assert candle.open_price == 100.0
        assert candle.high_price == 110.0
        assert candle.low_price == 95.0
        assert candle.close_price == 105.0

    def test_price_history_range(self) -> None:
        """Verify price history returns correct time range."""
        import time

        from price_oracle.db import PriceStore
        from price_oracle.models import PriceRecord

        store = PriceStore(":memory:")
        store.init_schema()

        now = int(time.time())

        # Insert prices at different times
        for i in range(10):
            record = PriceRecord(
                asset="bitcoin",
                source="test",
                price=100.0 + i,
                timestamp=now - (i * 60),  # 1 minute apart
            )
            store.insert_price(record)

        # Query middle range
        from_ts = now - (7 * 60)
        to_ts = now - (3 * 60)
        history = store.get_price_history("bitcoin", from_ts, to_ts)

        # Should get prices from t-7m to t-3m (5 prices)
        assert len(history) == 5
        for record in history:
            assert from_ts <= record.timestamp <= to_ts
