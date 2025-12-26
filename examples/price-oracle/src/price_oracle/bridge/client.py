"""Bridge client for pushing prices to the validator.

This module demonstrates failure handling patterns for message-passing systems:
- Retry logic with exponential backoff for transient errors
- Status polling to track message lifecycle
- Error classification (retryable vs non-retryable)
- Timeout handling
"""

from __future__ import annotations

import asyncio
import logging
import time
from typing import TYPE_CHECKING, Optional

import httpx
from eth_abi import encode
from eth_hash.auto import keccak

from .types import MessageStatus, PushResult, is_retryable_error, stage_to_status

if TYPE_CHECKING:
    from ..models import PriceComparison

logger = logging.getLogger(__name__)


def compute_selector(function_sig: str) -> bytes:
    """Compute the 4-byte function selector from a function signature."""
    return keccak(function_sig.encode())[:4]


class BridgeClient:
    """Client for pushing price updates to the Bridge validator.

    This client demonstrates failure handling patterns for message-passing:

    1. Retry Logic: Automatic retries with exponential backoff for transient errors
    2. Status Polling: Wait for message completion with configurable timeout
    3. Error Classification: Distinguish retryable vs non-retryable failures
    4. Timeout Handling: Configurable timeouts for requests and polling
    """

    # Function signature for updatePrice(string,uint256,uint256)
    UPDATE_PRICE_SIG = "updatePrice(string,uint256,uint256)"
    UPDATE_PRICE_SELECTOR = compute_selector(UPDATE_PRICE_SIG)

    def __init__(
        self,
        validator_url: str,
        domain: str,
        max_retries: int = 3,
        retry_delay: float = 1.0,
        timeout: float = 30.0,
    ):
        """
        Initialize the bridge client.

        Args:
            validator_url: URL of the Bridge validator (e.g., http://localhost:8080)
            domain: Application domain as hex string (0x prefixed, 32 bytes)
            max_retries: Maximum number of retry attempts for retryable errors
            retry_delay: Initial delay between retries (doubles with each attempt)
            timeout: HTTP request timeout in seconds
        """
        self.validator_url = validator_url.rstrip("/")
        self.domain = domain
        self.max_retries = max_retries
        self.retry_delay = retry_delay
        self.timeout = timeout
        self._client = httpx.AsyncClient(timeout=timeout)
        self._nonce = 1

    def _get_nonce(self) -> int:
        """Get and increment the nonce."""
        nonce = self._nonce
        self._nonce += 1
        return nonce

    def _encode_calldata(self, asset: str, price_scaled: int, timestamp: int) -> str:
        """
        Encode calldata for updatePrice(string,uint256,uint256).

        Uses proper ABI encoding via eth-abi library.
        """
        # Encode the function arguments using proper ABI encoding
        encoded_args = encode(
            ["string", "uint256", "uint256"],
            [asset, price_scaled, timestamp],
        )

        # Prepend the function selector
        calldata = self.UPDATE_PRICE_SELECTOR + encoded_args
        return "0x" + calldata.hex()

    async def push_price(
        self,
        asset: str,
        price: float,
        timestamp: Optional[int] = None,
        sources_count: Optional[int] = None,
        price_diff_bps: Optional[int] = None,
    ) -> Optional[str]:
        """
        Push a price update to the Bridge validator.

        Args:
            asset: Asset symbol (e.g., "bitcoin")
            price: Price in USD
            timestamp: Optional timestamp (defaults to current time)
            sources_count: Number of sources that were aggregated
            price_diff_bps: Price difference across sources in basis points

        Returns:
            Message ID if successful, None otherwise
        """
        if timestamp is None:
            timestamp = int(time.time())

        price_scaled = int(price * 10**18)

        nonce = self._get_nonce()
        now = int(time.time())

        calldata = self._encode_calldata(asset, price_scaled, timestamp)

        # Build metadata that matches the schema
        metadata: dict = {
            "reason": "price_update",
            "asset": asset,
            "price_scaled": str(price_scaled),
            "timestamp": timestamp,
            "source": "price-oracle",
        }

        # Include validation metadata if available
        if sources_count is not None:
            metadata["sources_count"] = sources_count
        if price_diff_bps is not None:
            metadata["price_diff_bps"] = price_diff_bps

        message = {
            "messageType": self.UPDATE_PRICE_SIG,
            "calldata": calldata,
            "metadata": metadata,
            "nonce": nonce,
            "timestamp": now,
            "domain": self.domain,
        }

        return await self._submit_message(message)

    async def push_validated_price(
        self,
        comparison: "PriceComparison",
        timestamp: Optional[int] = None,
    ) -> Optional[str]:
        """
        Push a validated price comparison to the Bridge validator.

        This method should be used after prices have been compared across sources
        and validated for consistency.

        Args:
            comparison: PriceComparison result from compare_prices()
            timestamp: Optional timestamp (defaults to current time)

        Returns:
            Message ID if successful, None otherwise
        """
        return await self.push_price(
            asset=comparison.asset,
            price=comparison.avg_price,
            timestamp=timestamp,
            sources_count=len(comparison.sources),
            price_diff_bps=comparison.difference_bps,
        )

    async def _submit_message(self, message: dict) -> Optional[str]:
        """Submit a message to the Bridge validator."""
        try:
            response = await self._client.post(
                f"{self.validator_url}/messages",
                json=message,
            )

            if response.status_code == 200:
                data = response.json()
                if data.get("status") == "accepted":
                    return data.get("messageId")
                logger.error(f"Bridge rejected message: {data.get('error')}")
                return None

            logger.error(f"Bridge returned {response.status_code}: {response.text}")
            return None

        except httpx.HTTPError as e:
            logger.error(f"Bridge connection error: {e}")
            return None

    # -------------------------------------------------------------------------
    # Failure Handling Methods - Demonstrating message-passing challenges
    # -------------------------------------------------------------------------

    async def push_price_with_retry(
        self,
        asset: str,
        price: float,
        timestamp: Optional[int] = None,
        sources_count: Optional[int] = None,
        price_diff_bps: Optional[int] = None,
    ) -> PushResult:
        """Push a price update with automatic retry for transient errors.

        This method demonstrates the retry pattern needed for message-passing:
        - Transient errors (network issues, temporary service unavailability)
          are automatically retried with exponential backoff
        - Non-retryable errors (validation failures, invalid data) fail immediately
        - The result includes detailed error information for debugging

        In contrast, SQLite transactions either succeed or fail atomically
        with no need for client-side retry logic.
        """
        if timestamp is None:
            timestamp = int(time.time())

        price_scaled = int(price * 10**18)
        attempts = 0
        delay = self.retry_delay

        while attempts < self.max_retries:
            attempts += 1

            # Build fresh message with new nonce for each attempt
            nonce = self._get_nonce()
            now = int(time.time())
            calldata = self._encode_calldata(asset, price_scaled, timestamp)

            metadata: dict = {
                "reason": "price_update",
                "asset": asset,
                "price_scaled": str(price_scaled),
                "timestamp": timestamp,
                "source": "price-oracle",
            }
            if sources_count is not None:
                metadata["sources_count"] = sources_count
            if price_diff_bps is not None:
                metadata["price_diff_bps"] = price_diff_bps

            message = {
                "messageType": self.UPDATE_PRICE_SIG,
                "calldata": calldata,
                "metadata": metadata,
                "nonce": nonce,
                "timestamp": now,
                "domain": self.domain,
            }

            result = await self._submit_message_detailed(message)
            result.attempts = attempts

            if result.success:
                return result

            if not result.is_retryable:
                logger.warning(
                    f"Non-retryable error after {attempts} attempt(s): {result.error_code}"
                )
                return result

            if attempts < self.max_retries:
                logger.info(
                    f"Retryable error, attempt {attempts}/{self.max_retries}, "
                    f"waiting {delay:.1f}s: {result.error_code}"
                )
                await asyncio.sleep(delay)
                delay *= 2  # Exponential backoff

        return PushResult(
            success=False,
            error_code="MAX_RETRIES_EXCEEDED",
            error_message=f"Failed after {attempts} attempts",
            is_retryable=False,
            attempts=attempts,
        )

    async def _submit_message_detailed(self, message: dict) -> PushResult:
        """Submit a message and return detailed result with error classification."""
        try:
            response = await self._client.post(
                f"{self.validator_url}/messages",
                json=message,
            )

            data = response.json()

            if response.status_code == 200 and data.get("status") == "accepted":
                return PushResult(
                    success=True,
                    message_id=data.get("messageId"),
                    signature=data.get("signature"),
                    is_retryable=False,
                    attempts=1,
                )

            # Extract error details
            error = data.get("error", {})
            error_code = error.get("code") if isinstance(error, dict) else str(error)
            error_message = (
                error.get("message") if isinstance(error, dict) else str(error)
            )

            return PushResult(
                success=False,
                error_code=error_code,
                error_message=error_message,
                is_retryable=is_retryable_error(error_code),
                attempts=1,
            )

        except httpx.TimeoutException:
            return PushResult(
                success=False,
                error_code="TIMEOUT",
                error_message="Request timed out",
                is_retryable=True,
                attempts=1,
            )
        except httpx.ConnectError:
            return PushResult(
                success=False,
                error_code="CONNECTION_ERROR",
                error_message="Failed to connect to validator",
                is_retryable=True,
                attempts=1,
            )
        except httpx.HTTPError as e:
            return PushResult(
                success=False,
                error_code="HTTP_ERROR",
                error_message=str(e),
                is_retryable=True,
                attempts=1,
            )

    async def get_message_status(self, message_id: str) -> MessageStatus:
        """Get the current status of a message.

        Queries the validator for the on-chain message stage.

        This demonstrates a key difference from SQLite:
        - SQLite: Transaction either committed or not (binary)
        - Message-passing: 9 possible stages, must poll to track progress
        """
        try:
            response = await self._client.get(
                f"{self.validator_url}/messages/{message_id}",
            )

            if response.status_code == 200:
                data = response.json()
                return MessageStatus(
                    message_id=message_id,
                    stage=data.get("stage", 0),
                    status=data.get("status", stage_to_status(data.get("stage", 0))),
                    executed=data.get("executed", False),
                    signatures_collected=data.get("signaturesCollected", 0),
                    signature_threshold=data.get("signatureThreshold", 1),
                )

            # Message not found or error
            return MessageStatus(
                message_id=message_id,
                stage=0,
                status="not_initialized",
                executed=False,
            )

        except httpx.HTTPError:
            return MessageStatus(
                message_id=message_id,
                stage=0,
                status="unknown",
                executed=False,
            )

    async def wait_for_completion(
        self,
        message_id: str,
        timeout: float = 60.0,
        poll_interval: float = 2.0,
    ) -> MessageStatus:
        """Wait for a message to reach a terminal state.

        Polls the validator until the message is completed, failed, or expired,
        or until the timeout is reached.

        This demonstrates the polling pattern required for message-passing:
        - No push notifications: client must poll for status updates
        - Timeout handling: must decide when to give up
        - Multiple terminal states: success, failure, or expiration

        In contrast, SQLite transactions complete synchronously.

        Args:
            message_id: The message ID to track
            timeout: Maximum time to wait in seconds
            poll_interval: Time between status checks in seconds

        Returns:
            Final MessageStatus (may not be terminal if timeout reached)
        """
        start_time = time.time()
        last_status = None

        while (time.time() - start_time) < timeout:
            status = await self.get_message_status(message_id)

            # Log stage transitions
            if last_status is None or status.stage != last_status.stage:
                logger.info(
                    f"Message {message_id[:10]}... stage: {status.stage} ({status.status})"
                )
                last_status = status

            if status.is_terminal:
                return status

            await asyncio.sleep(poll_interval)

        # Timeout reached
        logger.warning(
            f"Timeout waiting for message {message_id[:10]}... "
            f"(last stage: {last_status.stage if last_status else 'unknown'})"
        )
        return last_status or MessageStatus(
            message_id=message_id,
            stage=0,
            status="timeout",
            executed=False,
        )

    async def close(self) -> None:
        """Close the HTTP client."""
        await self._client.aclose()
