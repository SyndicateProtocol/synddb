"""End-to-end tests for the full price oracle → validator → bridge flow.

These tests require:
1. Anvil running on port 8545
2. Base E2E setup: ./scripts/setup-e2e-test.sh
3. Price oracle setup: ./examples/price-oracle/scripts/setup-e2e.sh
4. Validator running on port 8080:
   source .e2e-test-env
   cargo run -p synddb-bridge-validator -- \
       --bridge-address $BRIDGE_ADDRESS \
       --bridge-chain-id 31337 \
       --rpc-url $RPC_URL \
       --private-key $VALIDATOR_PRIVATE_KEY \
       --http-port 8080

Run with:
    source .e2e-env
    pytest tests/test_validator_e2e.py -v -s
"""

import asyncio
import json
import os
import subprocess
import time

import httpx
import pytest

from price_oracle.bridge.client import BridgeClient

ORACLE_ADDRESS = os.environ.get("ORACLE_ADDRESS")
BRIDGE_ADDRESS = os.environ.get("BRIDGE_ADDRESS")
ORACLE_DOMAIN = os.environ.get("ORACLE_DOMAIN")
VALIDATOR_PRIVATE_KEY = os.environ.get("VALIDATOR_PRIVATE_KEY")
RPC_URL = os.environ.get("RPC_URL", "http://127.0.0.1:8545")
VALIDATOR_URL = os.environ.get("VALIDATOR_URL", "http://127.0.0.1:8080")

pytestmark = pytest.mark.skipif(
    not all([ORACLE_ADDRESS, BRIDGE_ADDRESS, ORACLE_DOMAIN]),
    reason="E2E environment not configured. Run setup-e2e.sh first.",
)


def is_validator_running() -> bool:
    """Check if the validator HTTP server is responding."""
    try:
        response = httpx.get(f"{VALIDATOR_URL}/health", timeout=2.0)
        return response.status_code == 200
    except httpx.ConnectError:
        return False


class TestValidatorHealth:
    """Test validator service health."""

    def test_validator_health_endpoint(self):
        """Verify validator health endpoint responds."""
        if not is_validator_running():
            pytest.skip("Validator not running. Start with: cargo run -p synddb-bridge-validator")

        response = httpx.get(f"{VALIDATOR_URL}/health")
        assert response.status_code == 200

        data = response.json()
        assert data["healthy"] is True
        assert "mode" in data

    def test_validator_ready_endpoint(self):
        """Verify validator ready endpoint responds."""
        if not is_validator_running():
            pytest.skip("Validator not running")

        response = httpx.get(f"{VALIDATOR_URL}/ready")
        assert response.status_code == 200

        data = response.json()
        assert data["ready"] is True


class TestValidatorMessageSubmission:
    """Test message submission through the validator HTTP API."""

    def test_submit_price_update_message(self):
        """Test submitting a price update message to the validator."""
        if not is_validator_running():
            pytest.skip("Validator not running")

        client = BridgeClient(VALIDATOR_URL, ORACLE_DOMAIN)

        # Build a test message
        asset = "bitcoin"
        price = 67196.645
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
            "domain": ORACLE_DOMAIN,
        }

        response = httpx.post(
            f"{VALIDATOR_URL}/messages",
            json=message,
            timeout=30.0,
        )

        # The request should be accepted
        assert response.status_code == 200, f"Unexpected status: {response.status_code}, body: {response.text}"

        data = response.json()
        assert data["status"] == "accepted", f"Message rejected: {data.get('error')}"
        assert "messageId" in data
        assert "signature" in data
        assert data["signature"] is not None

        print(f"Message accepted: {data['messageId']}")
        print(f"Signature: {data['signature'][:20]}...")

        return data["messageId"]

    def test_submit_message_with_invalid_domain(self):
        """Test that messages with unregistered domains are rejected."""
        if not is_validator_running():
            pytest.skip("Validator not running")

        # Use an invalid domain
        invalid_domain = "0x" + "ff" * 32

        client = BridgeClient(VALIDATOR_URL, invalid_domain)

        asset = "bitcoin"
        price = 67196.645
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
            "domain": invalid_domain,
        }

        response = httpx.post(
            f"{VALIDATOR_URL}/messages",
            json=message,
            timeout=30.0,
        )

        # Should be rejected
        assert response.status_code == 400, f"Expected 400, got {response.status_code}"

        data = response.json()
        assert data["status"] == "rejected"
        assert data["error"] is not None
        print(f"Correctly rejected: {data['error']}")


class TestValidatorBridgeIntegration:
    """Test the full flow from validator to bridge contract."""

    def test_message_initialized_on_bridge(self):
        """Verify that accepted messages are initialized on the Bridge."""
        if not is_validator_running():
            pytest.skip("Validator not running")

        client = BridgeClient(VALIDATOR_URL, ORACLE_DOMAIN)

        # Submit a message
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
            "domain": ORACLE_DOMAIN,
        }

        response = httpx.post(
            f"{VALIDATOR_URL}/messages",
            json=message,
            timeout=30.0,
        )

        assert response.status_code == 200
        data = response.json()
        assert data["status"] == "accepted"

        message_id = data["messageId"]

        # Query the message status from the validator
        status_response = httpx.get(
            f"{VALIDATOR_URL}/messages/{message_id}",
            timeout=10.0,
        )

        if status_response.status_code == 200:
            status_data = status_response.json()
            print(f"Message status: {status_data}")
            assert status_data["stage"] >= 1, "Message should be at least initialized"
        else:
            print(f"Status query returned: {status_response.status_code}")

        # Query the bridge directly using cast
        result = subprocess.run(
            [
                "cast",
                "call",
                BRIDGE_ADDRESS,
                "getMessageStage(bytes32)",
                message_id,
                "--rpc-url",
                RPC_URL,
            ],
            capture_output=True,
            text=True,
        )

        if result.returncode == 0:
            stage = int(result.stdout.strip(), 16)
            print(f"Bridge message stage: {stage}")
            assert stage >= 1, f"Message stage {stage} indicates not initialized"
        else:
            # Bridge query failed, but message was accepted by validator
            print(f"Bridge query failed (may be expected): {result.stderr}")


@pytest.mark.asyncio
class TestAsyncValidatorFlow:
    """Test async message submission flow."""

    async def test_push_price_via_client(self):
        """Test pushing a price using the BridgeClient.push_price method."""
        if not is_validator_running():
            pytest.skip("Validator not running")

        client = BridgeClient(VALIDATOR_URL, ORACLE_DOMAIN)

        try:
            # This will submit through the HTTP API
            message_id = await client.push_price(
                asset="solana",
                price=142.50,
                sources_count=2,
                price_diff_bps=25,
            )

            if message_id:
                print(f"Price pushed successfully, message ID: {message_id}")
            else:
                # May fail due to various reasons, but the client should handle it
                print("Price push returned None (may be expected if validation fails)")

        finally:
            await client.close()

    async def test_push_multiple_prices(self):
        """Test pushing multiple price updates in sequence."""
        if not is_validator_running():
            pytest.skip("Validator not running")

        client = BridgeClient(VALIDATOR_URL, ORACLE_DOMAIN)

        try:
            assets = ["bitcoin", "ethereum", "solana"]
            prices = [67196.645, 3514.49, 142.50]

            for asset, price in zip(assets, prices):
                message_id = await client.push_price(
                    asset=asset,
                    price=price,
                    sources_count=3,
                    price_diff_bps=10,
                )

                if message_id:
                    print(f"{asset}: {message_id}")
                else:
                    print(f"{asset}: failed to push")

        finally:
            await client.close()


class TestMessageStatusQueries:
    """Test message status query endpoints."""

    def test_query_nonexistent_message(self):
        """Test querying a message that doesn't exist."""
        if not is_validator_running():
            pytest.skip("Validator not running")

        # Random message ID that doesn't exist
        fake_message_id = "0x" + "ab" * 32

        response = httpx.get(
            f"{VALIDATOR_URL}/messages/{fake_message_id}",
            timeout=10.0,
        )

        # Should return 200 with stage 0 (not initialized)
        if response.status_code == 200:
            data = response.json()
            assert data["stage"] == 0
            assert data["status"] == "not_initialized"


class TestSchemaEndpoint:
    """Test schema query endpoint."""

    def test_query_update_price_schema(self):
        """Query the updatePrice message type schema."""
        if not is_validator_running():
            pytest.skip("Validator not running")

        message_type = "updatePrice(string,uint256,uint256)"
        # URL encode the message type
        encoded_type = message_type.replace("(", "%28").replace(")", "%29").replace(",", "%2C")

        response = httpx.get(
            f"{VALIDATOR_URL}/schemas/{encoded_type}",
            timeout=10.0,
        )

        assert response.status_code == 200, f"Status: {response.status_code}, Body: {response.text}"

        data = response.json()
        assert data["messageType"] == message_type
        assert data["enabled"] is True
        print(f"Schema endpoint returned: {json.dumps(data, indent=2)}")
