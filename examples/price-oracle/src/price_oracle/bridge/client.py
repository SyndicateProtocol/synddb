"""Bridge client for pushing prices to the validator."""

from __future__ import annotations

import logging
import time
from typing import TYPE_CHECKING, Optional

import httpx
from eth_abi import encode
from eth_hash.auto import keccak

if TYPE_CHECKING:
    from ..models import PriceComparison

logger = logging.getLogger(__name__)


def compute_selector(function_sig: str) -> bytes:
    """Compute the 4-byte function selector from a function signature."""
    return keccak(function_sig.encode())[:4]


class BridgeClient:
    """Client for pushing price updates to the Bridge validator."""

    # Function signature for updatePrice(string,uint256,uint256)
    UPDATE_PRICE_SIG = "updatePrice(string,uint256,uint256)"
    UPDATE_PRICE_SELECTOR = compute_selector(UPDATE_PRICE_SIG)

    def __init__(self, validator_url: str, domain: str):
        """
        Initialize the bridge client.

        Args:
            validator_url: URL of the Bridge validator (e.g., http://localhost:8080)
            domain: Application domain as hex string (0x prefixed, 32 bytes)
        """
        self.validator_url = validator_url.rstrip("/")
        self.domain = domain
        self._client = httpx.AsyncClient(timeout=30.0)
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

    async def close(self) -> None:
        """Close the HTTP client."""
        await self._client.aclose()
