"""Chain Monitor integration for the Price Oracle

This module provides handlers for blockchain events from the PriceOracle contract:
- PriceRequested: Inbound message indicating someone wants a price update

The chain monitor can operate in two modes:
1. Polling mode: Uses web3.py to poll for events (simpler, for development)
2. External mode: Receives events from the Rust chain monitor via channel (production)

Architecture:
    PriceOracle.sol (PriceRequested event)
            |
            v
    Chain Monitor (WebSocket or polling)
            |
            v
    PriceRequestHandler (decodes event)
            |
            v (MessageClient.push)
    Sequencer Message Queue
            |
            v (MessageClient.get_messages)
    Application processes and acks
"""

import logging
import time
from dataclasses import dataclass
from typing import Optional

try:
    from web3 import Web3

    HAS_WEB3 = True
except ImportError:
    HAS_WEB3 = False

# Import MessageClient from SDK
import sys
import os
sys.path.insert(0, os.path.join(os.path.dirname(__file__), '..', '..', '..', 'crates', 'synddb-client', 'bindings', 'python'))
from synddb import MessageClient

logger = logging.getLogger(__name__)


# PriceOracle contract ABI (minimal, just the events we need)
PRICE_ORACLE_ABI = [
    {
        "anonymous": False,
        "inputs": [
            {"indexed": True, "name": "requestId", "type": "bytes32"},
            {"indexed": True, "name": "asset", "type": "string"},
            {"indexed": True, "name": "requester", "type": "address"},
            {"indexed": False, "name": "maxAge", "type": "uint256"},
        ],
        "name": "PriceRequested",
        "type": "event",
    },
    {
        "anonymous": False,
        "inputs": [
            {"indexed": True, "name": "asset", "type": "string"},
            {"indexed": False, "name": "price", "type": "uint256"},
            {"indexed": False, "name": "timestamp", "type": "uint256"},
            {"indexed": True, "name": "source", "type": "address"},
        ],
        "name": "PriceUpdated",
        "type": "event",
    },
    {
        "anonymous": False,
        "inputs": [
            {"indexed": True, "name": "requestId", "type": "bytes32"},
            {"indexed": True, "name": "asset", "type": "string"},
            {"indexed": False, "name": "price", "type": "uint256"},
        ],
        "name": "PriceRequestFulfilled",
        "type": "event",
    },
]

# Event signatures (keccak256 of event signature)
# PriceRequested(bytes32 indexed requestId, string indexed asset, address indexed requester, uint256 maxAge)
PRICE_REQUESTED_TOPIC = Web3.keccak(
    text="PriceRequested(bytes32,string,address,uint256)"
).hex() if HAS_WEB3 else "0x..."

# Mapping of keccak256(asset_name) -> asset_name for indexed string recovery
# When a string is indexed in an event, only its hash is stored in topics
KNOWN_ASSET_HASHES = {}
if HAS_WEB3:
    for asset in ["BTC", "ETH", "SOL", "USDC", "USDT"]:
        asset_hash = Web3.keccak(text=asset).hex()
        KNOWN_ASSET_HASHES[asset_hash] = asset


@dataclass
class ChainMonitorConfig:
    """Configuration for the chain monitor."""

    rpc_url: str  # HTTP or WebSocket RPC URL
    contract_address: str  # PriceOracle contract address
    start_block: int = 0  # Block to start scanning from (0 = latest)
    poll_interval: int = 5  # Seconds between polls (for polling mode)
    confirmations: int = 1  # Number of confirmations before processing


class PriceRequestHandler:
    """Handler for PriceRequested events from the PriceOracle contract.

    When a user calls PriceOracle.requestPrice(), this handler captures
    the event and pushes it to the sequencer's message queue via MessageClient.
    """

    def __init__(self, sequencer_url: str):
        """Initialize the handler.

        Args:
            sequencer_url: URL of the sequencer (e.g., 'http://localhost:8433')
        """
        self.client = MessageClient(sequencer_url)
        self.processed_count = 0

    def handle_event(self, event: dict) -> bool:
        """Process a PriceRequested event.

        Pushes the event to the sequencer's message queue via MessageClient.push().

        Args:
            event: Decoded event data with keys:
                - requestId: bytes32 request ID
                - asset: string asset symbol
                - requester: address of requester
                - maxAge: uint256 max acceptable age
                - blockNumber: block where event was emitted
                - transactionHash: tx hash
                - logIndex: index within tx

        Returns:
            True if event was pushed successfully
        """
        try:
            result = self.client.push(
                message_id=event["requestId"],
                message_type="price_request",
                payload={
                    "asset": event["asset"],
                    "requester": event["requester"],
                    "max_age": event["maxAge"],
                    "log_index": event.get("logIndex"),
                },
                sender=event["requester"],
                tx_hash=event["transactionHash"],
                block_number=event["blockNumber"],
                confirmations=0,
            )

            if result.get("id"):
                self.processed_count += 1
                logger.info(
                    f"Pushed price request to sequencer: id={result['id']}, "
                    f"asset={event['asset']}, requester={event['requester']}"
                )
                return True
            return False

        except Exception as e:
            logger.error(f"Error pushing PriceRequested event to sequencer: {e}")
            return False


class PollingChainMonitor:
    """Simple polling-based chain monitor for development.

    This uses web3.py to poll for events. For production, use the
    Rust-based chain monitor with WebSocket subscriptions.
    """

    def __init__(
        self,
        config: ChainMonitorConfig,
        handler: PriceRequestHandler,
    ):
        """Initialize the polling monitor.

        Args:
            config: Monitor configuration
            handler: Handler for PriceRequested events
        """
        if not HAS_WEB3:
            raise ImportError(
                "web3 package required for polling mode. Install with: pip install web3"
            )

        self.config = config
        self.handler = handler
        self.w3 = Web3(Web3.HTTPProvider(config.rpc_url))
        self.contract = self.w3.eth.contract(
            address=Web3.to_checksum_address(config.contract_address),
            abi=PRICE_ORACLE_ABI,
        )
        self.last_block = config.start_block or self.w3.eth.block_number

    def poll_once(self) -> int:
        """Poll for new events once.

        Returns:
            Number of events processed
        """
        current_block = self.w3.eth.block_number
        safe_block = current_block - self.config.confirmations

        if safe_block <= self.last_block:
            return 0

        logger.debug(f"Scanning blocks {self.last_block + 1} to {safe_block}")

        # Get PriceRequested events
        try:
            events = self.contract.events.PriceRequested.get_logs(
                from_block=self.last_block + 1,
                to_block=safe_block,
            )
        except Exception as e:
            logger.error(f"Error fetching events: {e}")
            return 0

        processed = 0
        for event in events:
            # Indexed string params are stored as keccak256 hash (bytes)
            # Look up the original asset name from our known assets mapping
            asset_hash = "0x" + event.args.asset.hex()
            asset_name = KNOWN_ASSET_HASHES.get(asset_hash, f"<unknown:{asset_hash[:18]}>")

            # Ensure all values are JSON serializable strings
            event_data = {
                "requestId": "0x" + event.args.requestId.hex(),
                "asset": asset_name,
                "requester": str(event.args.requester),
                "maxAge": int(event.args.maxAge),
                "blockNumber": int(event.blockNumber),
                "transactionHash": "0x" + event.transactionHash.hex(),
                "logIndex": int(event.logIndex),
            }

            if self.handler.handle_event(event_data):
                processed += 1

        self.last_block = safe_block
        return processed

    def run(self, stop_event=None):
        """Run the polling loop.

        Args:
            stop_event: Optional threading.Event to signal stop
        """
        logger.info(
            f"Starting chain monitor (polling mode) for {self.config.contract_address}"
        )
        logger.info(f"Poll interval: {self.config.poll_interval}s")

        while True:
            if stop_event and stop_event.is_set():
                break

            try:
                count = self.poll_once()
                if count > 0:
                    logger.info(f"Processed {count} PriceRequested events")
            except Exception as e:
                logger.error(f"Error in poll loop: {e}")

            time.sleep(self.config.poll_interval)

        logger.info("Chain monitor stopped")


def decode_price_requested_log(log: dict) -> Optional[dict]:
    """Decode a raw log into a PriceRequested event.

    This is used when receiving raw logs from an external chain monitor
    (e.g., the Rust chain monitor).

    Args:
        log: Raw log dict with topics and data

    Returns:
        Decoded event dict or None if not a PriceRequested event
    """
    if not HAS_WEB3:
        logger.warning("web3 not available, cannot decode logs")
        return None

    topics = log.get("topics", [])
    if not topics:
        return None

    # Check if this is a PriceRequested event
    if topics[0].hex() if hasattr(topics[0], "hex") else topics[0] != PRICE_REQUESTED_TOPIC:
        return None

    try:
        # Decode indexed parameters from topics
        request_id = topics[1].hex() if hasattr(topics[1], "hex") else topics[1]
        # asset is indexed as a topic (keccak256 of the string)
        # We can't recover the original string from the hash
        asset_hash = topics[2].hex() if hasattr(topics[2], "hex") else topics[2]
        requester = "0x" + (topics[3].hex() if hasattr(topics[3], "hex") else topics[3])[-40:]

        # Decode non-indexed parameter from data
        data = log.get("data", "0x")
        if isinstance(data, bytes):
            data = data.hex()
        if data.startswith("0x"):
            data = data[2:]

        max_age = int(data, 16) if data else 0

        return {
            "requestId": request_id,
            "asset": f"<hash:{asset_hash[:16]}...>",  # We don't have the original string
            "requester": requester,
            "maxAge": max_age,
            "blockNumber": log.get("blockNumber", 0),
            "transactionHash": log.get("transactionHash", ""),
            "logIndex": log.get("logIndex"),
        }

    except Exception as e:
        logger.error(f"Error decoding PriceRequested log: {e}")
        return None


# For integration with Rust chain monitor via FFI/IPC
class ExternalEventReceiver:
    """Receives events from an external chain monitor (e.g., Rust).

    This is used in production where the Rust chain monitor handles
    WebSocket subscriptions and delivers events via channel/IPC.
    """

    def __init__(self, handler: PriceRequestHandler):
        """Initialize the receiver.

        Args:
            handler: Handler for processing events
        """
        self.handler = handler

    def on_event(self, event_type: str, event_data: dict) -> bool:
        """Called by external monitor when an event is received.

        Args:
            event_type: Type of event (e.g., "PriceRequested")
            event_data: Decoded event data

        Returns:
            True if event was handled successfully
        """
        if event_type == "PriceRequested":
            return self.handler.handle_event(event_data)
        else:
            logger.warning(f"Unknown event type: {event_type}")
            return False
