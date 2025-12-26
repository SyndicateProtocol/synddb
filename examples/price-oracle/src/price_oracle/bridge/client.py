"""Bridge client for pushing prices to the validator."""

import hashlib
import logging
import time
from typing import Optional

import httpx

logger = logging.getLogger(__name__)


class BridgeClient:
    """Client for pushing price updates to the Bridge validator."""

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

        This is a simplified ABI encoding. In production, use proper ABI encoding.
        """
        function_sig = "updatePrice(string,uint256,uint256)"
        selector = hashlib.sha3_256(function_sig.encode()).digest()[:4].hex()

        asset_bytes = asset.encode("utf-8")
        asset_hex = asset_bytes.hex().ljust(64, "0")

        price_hex = hex(price_scaled)[2:].zfill(64)
        timestamp_hex = hex(timestamp)[2:].zfill(64)

        offset_hex = "0000000000000000000000000000000000000000000000000000000000000060"
        length_hex = hex(len(asset_bytes))[2:].zfill(64)

        return f"0x{selector}{offset_hex}{price_hex}{timestamp_hex}{length_hex}{asset_hex}"

    async def push_price(
        self,
        asset: str,
        price: float,
        timestamp: Optional[int] = None,
    ) -> Optional[str]:
        """
        Push a price update to the Bridge validator.

        Args:
            asset: Asset symbol (e.g., "bitcoin")
            price: Price in USD
            timestamp: Optional timestamp (defaults to current time)

        Returns:
            Message ID if successful, None otherwise
        """
        if timestamp is None:
            timestamp = int(time.time())

        price_scaled = int(price * 10**18)

        nonce = self._get_nonce()
        now = int(time.time())

        calldata = self._encode_calldata(asset, price_scaled, timestamp)

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
            "domain": self.domain,
        }

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
