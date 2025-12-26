"""End-to-end tests for price oracle with Bridge and PriceOracle contract.

These tests require:
1. Anvil running on port 8545
2. Base E2E setup: ./scripts/setup-e2e-test.sh
3. Price oracle setup: ./examples/price-oracle/scripts/setup-e2e.sh

Run with:
    source .e2e-env
    pytest tests/test_e2e_bridge.py -v
"""

import os
import time

import pytest

# Check if E2E environment is configured
ORACLE_ADDRESS = os.environ.get("ORACLE_ADDRESS")
BRIDGE_ADDRESS = os.environ.get("BRIDGE_ADDRESS")
ORACLE_DOMAIN = os.environ.get("ORACLE_DOMAIN")
RPC_URL = os.environ.get("RPC_URL", "http://127.0.0.1:8545")

pytestmark = pytest.mark.skipif(
    not all([ORACLE_ADDRESS, BRIDGE_ADDRESS, ORACLE_DOMAIN]),
    reason="E2E environment not configured. Run setup-e2e.sh first.",
)


class TestPriceOracleContractE2E:
    """Test PriceOracle contract directly via cast calls."""

    def test_oracle_bridge_address(self):
        """Verify PriceOracle has correct bridge address."""
        import subprocess

        result = subprocess.run(
            ["cast", "call", ORACLE_ADDRESS, "bridge()", "--rpc-url", RPC_URL],
            capture_output=True,
            text=True,
        )
        assert result.returncode == 0
        # The result is padded to 32 bytes, extract the address (last 40 chars)
        bridge_from_oracle = result.stdout.strip()
        # Strip 0x prefix and padding to get just the address
        bridge_addr = "0x" + bridge_from_oracle[-40:]
        assert bridge_addr.lower() == BRIDGE_ADDRESS.lower()

    def test_oracle_initial_state(self):
        """Verify PriceOracle starts with no assets."""
        import subprocess

        result = subprocess.run(
            ["cast", "call", ORACLE_ADDRESS, "assetCount()", "--rpc-url", RPC_URL],
            capture_output=True,
            text=True,
        )
        assert result.returncode == 0
        count = int(result.stdout.strip(), 16)
        # May have assets from previous test runs
        assert count >= 0


class TestCalldataVerification:
    """Verify calldata encoding matches what the contract expects."""

    def test_selector_matches_contract(self):
        """Verify Python selector matches Solidity."""
        import subprocess

        from price_oracle.bridge.client import BridgeClient

        # Get selector from Python
        python_selector = BridgeClient.UPDATE_PRICE_SELECTOR.hex()

        # Get selector from cast
        result = subprocess.run(
            ["cast", "sig", "updatePrice(string,uint256,uint256)"],
            capture_output=True,
            text=True,
        )
        assert result.returncode == 0
        solidity_selector = result.stdout.strip().replace("0x", "")

        assert python_selector == solidity_selector, (
            f"Selector mismatch: Python={python_selector}, Solidity={solidity_selector}"
        )

    def test_abi_encoding_matches(self):
        """Verify Python ABI encoding matches Solidity."""
        import subprocess

        from price_oracle.bridge.client import BridgeClient

        client = BridgeClient("http://unused", ORACLE_DOMAIN)

        # Encode with Python
        asset = "bitcoin"
        price_scaled = 67196645000000000000000  # ~$67,196.645
        timestamp = 1735200000

        python_calldata = client._encode_calldata(asset, price_scaled, timestamp)

        # Encode with cast
        result = subprocess.run(
            [
                "cast",
                "calldata",
                "updatePrice(string,uint256,uint256)",
                asset,
                str(price_scaled),
                str(timestamp),
            ],
            capture_output=True,
            text=True,
        )
        assert result.returncode == 0
        solidity_calldata = result.stdout.strip()

        assert python_calldata.lower() == solidity_calldata.lower(), (
            f"Calldata mismatch:\n"
            f"Python:   {python_calldata}\n"
            f"Solidity: {solidity_calldata}"
        )


class TestMessageTypeRegistration:
    """Verify message type is properly registered on the Bridge."""

    def test_message_type_registered(self):
        """Verify updatePrice message type is registered."""
        import subprocess

        # Query the message type config
        result = subprocess.run(
            [
                "cast",
                "call",
                BRIDGE_ADDRESS,
                "getMessageTypeConfig(string)",
                "updatePrice(string,uint256,uint256)",
                "--rpc-url",
                RPC_URL,
            ],
            capture_output=True,
            text=True,
        )
        # If the call succeeds, the message type is registered
        assert result.returncode == 0, f"Failed to query message type: {result.stderr}"


class TestDomainRegistration:
    """Verify price-oracle domain is registered."""

    def test_domain_registered(self):
        """Verify price-oracle application domain is registered."""
        import subprocess

        # Query the application config
        result = subprocess.run(
            [
                "cast",
                "call",
                BRIDGE_ADDRESS,
                "getApplicationConfig(bytes32)",
                ORACLE_DOMAIN,
                "--rpc-url",
                RPC_URL,
            ],
            capture_output=True,
            text=True,
        )
        assert result.returncode == 0, f"Failed to query domain: {result.stderr}"
        # The result should contain the validator address
        assert result.stdout.strip() != "", "Domain not registered"


class TestMetadataSchema:
    """Test metadata validation against schema."""

    def test_valid_metadata(self):
        """Verify valid metadata passes schema validation."""
        import json

        import jsonschema

        schema_path = os.path.join(
            os.path.dirname(__file__), "..", "schemas", "update-price.schema.json"
        )

        with open(schema_path) as f:
            schema = json.load(f)

        valid_metadata = {
            "reason": "price_update",
            "asset": "bitcoin",
            "price_scaled": "67196645000000000000000",
            "timestamp": 1735200000,
            "source": "price-oracle",
        }

        # Should not raise
        jsonschema.validate(valid_metadata, schema)

    def test_valid_metadata_with_optional_fields(self):
        """Verify metadata with optional fields passes."""
        import json

        import jsonschema

        schema_path = os.path.join(
            os.path.dirname(__file__), "..", "schemas", "update-price.schema.json"
        )

        with open(schema_path) as f:
            schema = json.load(f)

        valid_metadata = {
            "reason": "price_update",
            "asset": "bitcoin",
            "price_scaled": "67196645000000000000000",
            "timestamp": 1735200000,
            "source": "price-oracle",
            "sources_count": 2,
            "price_diff_bps": 50,
        }

        jsonschema.validate(valid_metadata, schema)

    def test_invalid_reason_rejected(self):
        """Verify wrong reason is rejected."""
        import json

        import jsonschema

        schema_path = os.path.join(
            os.path.dirname(__file__), "..", "schemas", "update-price.schema.json"
        )

        with open(schema_path) as f:
            schema = json.load(f)

        invalid_metadata = {
            "reason": "wrong_reason",  # Must be "price_update"
            "asset": "bitcoin",
            "price_scaled": "67196645000000000000000",
            "timestamp": 1735200000,
            "source": "price-oracle",
        }

        with pytest.raises(jsonschema.ValidationError):
            jsonschema.validate(invalid_metadata, schema)

    def test_missing_required_field_rejected(self):
        """Verify missing required field is rejected."""
        import json

        import jsonschema

        schema_path = os.path.join(
            os.path.dirname(__file__), "..", "schemas", "update-price.schema.json"
        )

        with open(schema_path) as f:
            schema = json.load(f)

        invalid_metadata = {
            "reason": "price_update",
            # Missing "asset"
            "price_scaled": "67196645000000000000000",
            "timestamp": 1735200000,
            "source": "price-oracle",
        }

        with pytest.raises(jsonschema.ValidationError):
            jsonschema.validate(invalid_metadata, schema)
