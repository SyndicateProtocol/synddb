"""Bridge integration for the Price Oracle

This module handles bidirectional communication with the PriceOracle smart contract:

Push Model (Outbound):
    - Oracle fetches prices from APIs
    - Writes price update message to message_log
    - Sequencer picks up and submits to Bridge.sol -> PriceOracle.updatePrice()

Pull Model (Inbound):
    - Contract emits PriceRequested event
    - Chain monitor captures event, inserts into inbound_message_log
    - Oracle processes request, fetches price, writes response to message_log
    - Sequencer submits response via Bridge.sol -> PriceOracle.fulfillPriceRequest()
"""

import json
import logging
import sqlite3
import time
import uuid
from dataclasses import dataclass
from typing import Optional

import cbor2
import requests

# Try to import eth_abi for ABI encoding (used for withdrawal calldata)
try:
    from eth_abi import encode as abi_encode
    HAS_ETH_ABI = True
except ImportError:
    HAS_ETH_ABI = False

logger = logging.getLogger(__name__)

# Global sequencer URL (set by caller)
_sequencer_url: Optional[str] = None
_price_oracle_address: Optional[str] = None
_relayer_url: Optional[str] = None


def set_sequencer_url(url: str) -> None:
    """Set the sequencer URL for withdrawal submissions."""
    global _sequencer_url
    _sequencer_url = url


def set_price_oracle_address(address: str) -> None:
    """Set the PriceOracle contract address for withdrawal targets."""
    global _price_oracle_address
    _price_oracle_address = address


def set_relayer_url(url: str) -> None:
    """Set the relayer URL for Bridge transaction submission."""
    global _relayer_url
    _relayer_url = url


def encode_fulfill_price_request(
    request_id: str, asset: str, price: int, timestamp: int
) -> bytes:
    """Encode the fulfillPriceRequest function call.

    Args:
        request_id: The bytes32 request ID (0x-prefixed hex string)
        asset: Asset symbol (e.g., "BTC")
        price: Price in 8-decimal fixed point
        timestamp: Unix timestamp

    Returns:
        ABI-encoded calldata for fulfillPriceRequest(bytes32, string, uint256, uint256)
    """
    if not HAS_ETH_ABI:
        raise RuntimeError("eth_abi package required for withdrawal encoding")

    # Function selector for fulfillPriceRequest(bytes32,string,uint256,uint256)
    # cast sig "fulfillPriceRequest(bytes32,string,uint256,uint256)" = 0xcb15f73f
    selector = bytes.fromhex("cb15f73f")

    # Parse request_id as bytes32
    request_id_bytes = bytes.fromhex(request_id[2:] if request_id.startswith("0x") else request_id)
    if len(request_id_bytes) != 32:
        raise ValueError(f"request_id must be 32 bytes, got {len(request_id_bytes)}")

    # Encode parameters
    encoded_params = abi_encode(
        ["bytes32", "string", "uint256", "uint256"],
        [request_id_bytes, asset, price, timestamp]
    )

    return selector + encoded_params


def submit_withdrawal_request(
    request_id: str,
    asset: str,
    price: int,
    timestamp: int,
    sequencer_url: Optional[str] = None,
    price_oracle_address: Optional[str] = None,
    relayer_url: Optional[str] = None,
) -> Optional[str]:
    """Submit a withdrawal request to the sequencer and relayer for bridge execution.

    This creates a Withdrawal message, gets it signed by the sequencer, then
    submits it to the relayer for Bridge contract execution.

    Args:
        request_id: The bytes32 request ID from PriceRequested event
        asset: Asset symbol
        price: Price in 8-decimal fixed point
        timestamp: Unix timestamp
        sequencer_url: Sequencer URL (uses global if not provided)
        price_oracle_address: Target contract address (uses global if not provided)
        relayer_url: Relayer URL (uses global if not provided)

    Returns:
        Sequence number if successful, None if failed
    """
    url = sequencer_url or _sequencer_url
    target = price_oracle_address or _price_oracle_address
    relayer = relayer_url or _relayer_url

    if not url:
        logger.warning("No sequencer URL configured, skipping withdrawal submission")
        return None

    if not target:
        logger.warning("No PriceOracle address configured, skipping withdrawal submission")
        return None

    if not HAS_ETH_ABI:
        logger.warning("eth_abi not available, skipping withdrawal submission")
        return None

    try:
        # Encode the fulfillPriceRequest calldata
        calldata = encode_fulfill_price_request(request_id, asset, price, timestamp)
        calldata_hex = "0x" + calldata.hex()

        # Create the withdrawal request
        withdrawal = {
            "request_id": request_id,
            "recipient": target,
            "amount": "0",  # No ETH transfer, just calldata execution
            "data": calldata,
        }

        # CBOR-encode the request
        cbor_data = cbor2.dumps(withdrawal)

        # Submit to sequencer
        response = requests.post(
            f"{url}/withdrawals",
            data=cbor_data,
            headers={"Content-Type": "application/cbor"},
            timeout=10,
        )

        if response.status_code not in (200, 201):
            logger.error(
                f"Failed to submit withdrawal: status={response.status_code}, "
                f"body={response.text[:200]}"
            )
            return None

        # Parse CBOR response
        result = cbor2.loads(response.content)
        sequence = result.get("sequence")
        seq_timestamp = result.get("timestamp")
        bridge_sig = result.get("bridge_signature", {})

        logger.info(
            f"Submitted withdrawal for price response: request_id={request_id[:16]}..., "
            f"sequence={sequence}"
        )

        # If relayer is configured, submit to Bridge via relayer
        if relayer and bridge_sig:
            try:
                # Get the message_id and signature from bridge_signature
                message_id = bridge_sig.get("message_id", request_id)
                bridge_signature = bridge_sig.get("signature", "")

                if bridge_signature:
                    relayer_request = {
                        "message_id": message_id,
                        "target_address": target,
                        "payload": calldata_hex,
                        "native_token_amount": "0",
                        "sequencer_signature": bridge_signature,
                        "sequence": sequence,
                        "timestamp": seq_timestamp,
                    }

                    relayer_response = requests.post(
                        f"{relayer}/submit-withdrawal",
                        json=relayer_request,
                        timeout=30,
                    )

                    if relayer_response.status_code in (200, 202):
                        relayer_result = relayer_response.json()
                        tx_hash = relayer_result.get("tx_hash")
                        status = relayer_result.get("status")
                        logger.info(
                            f"Relayer submitted withdrawal to Bridge: "
                            f"status={status}, tx_hash={tx_hash}"
                        )
                    else:
                        logger.warning(
                            f"Relayer failed to submit: status={relayer_response.status_code}, "
                            f"body={relayer_response.text[:200]}"
                        )
                else:
                    logger.warning("No bridge signature in sequencer response")
            except Exception as e:
                logger.error(f"Error submitting to relayer: {e}")
        elif not relayer:
            logger.debug("No relayer URL configured, skipping Bridge submission")

        return sequence

    except Exception as e:
        logger.error(f"Error submitting withdrawal request: {e}")
        return None


@dataclass
class PriceUpdate:
    """Represents a price update to be sent to the contract."""

    asset: str
    price: int  # Price in base units (8 decimals)
    timestamp: int  # Unix timestamp


@dataclass
class PriceRequest:
    """Represents a price request from the contract."""

    request_id: str  # bytes32 from contract
    asset: str
    requester: str  # Address
    max_age: int  # Seconds
    block_number: int
    tx_hash: str


def create_price_update_message(
    conn: sqlite3.Connection,
    asset: str,
    price: float,
    timestamp: int,
    trigger_event: str = "scheduled_update",
    trigger_id: Optional[str] = None,
) -> int:
    """Create an outbound message to update a price on-chain.

    Args:
        conn: Database connection
        asset: Asset symbol (e.g., "BTC")
        price: Price in USD (will be converted to 8 decimal fixed point)
        timestamp: Unix timestamp of the price
        trigger_event: What caused this update
        trigger_id: Reference to causing record

    Returns:
        Message ID from message_log
    """
    # Convert float price to 8 decimal fixed point
    price_fixed = int(price * 10**8)

    payload = json.dumps(
        {
            "asset": asset,
            "price": price_fixed,
            "timestamp": timestamp,
        }
    )

    # Create idempotency key from asset + timestamp
    idempotency_key = f"price_update:{asset}:{timestamp}"

    cursor = conn.execute(
        """
        INSERT INTO message_log (message_type, payload, idempotency_key, trigger_event, trigger_id)
        VALUES ('price_update', ?, ?, ?, ?)
        ON CONFLICT(idempotency_key) DO NOTHING
        """,
        (payload, idempotency_key, trigger_event, trigger_id),
    )
    conn.commit()

    if cursor.rowcount > 0:
        logger.info(f"Created price update message for {asset}: ${price:.2f}")
        return cursor.lastrowid
    else:
        logger.debug(f"Duplicate price update for {asset}:{timestamp}, skipped")
        return 0


def create_batch_price_update_message(
    conn: sqlite3.Connection,
    updates: list[PriceUpdate],
    trigger_event: str = "scheduled_batch",
) -> int:
    """Create an outbound message to batch update multiple prices.

    Args:
        conn: Database connection
        updates: List of PriceUpdate objects
        trigger_event: What caused this update

    Returns:
        Message ID from message_log
    """
    if not updates:
        return 0

    payload = json.dumps(
        {
            "assets": [u.asset for u in updates],
            "prices": [u.price for u in updates],
            "timestamps": [u.timestamp for u in updates],
        }
    )

    # Create idempotency key from hash of all updates
    key_parts = ":".join(f"{u.asset}:{u.timestamp}" for u in updates)
    idempotency_key = f"batch_price_update:{hash(key_parts)}"

    cursor = conn.execute(
        """
        INSERT INTO message_log (message_type, payload, idempotency_key, trigger_event)
        VALUES ('batch_price_update', ?, ?, ?)
        ON CONFLICT(idempotency_key) DO NOTHING
        """,
        (payload, idempotency_key, trigger_event),
    )
    conn.commit()

    if cursor.rowcount > 0:
        logger.info(f"Created batch price update for {len(updates)} assets")
        return cursor.lastrowid
    else:
        return 0


def create_price_response_message(
    conn: sqlite3.Connection,
    request_id: str,
    asset: str,
    price: float,
    timestamp: int,
    inbound_id: int,
) -> int:
    """Create a response message for a price request (pull model).

    Args:
        conn: Database connection
        request_id: The requestId from the PriceRequested event
        asset: Asset symbol
        price: Price in USD
        timestamp: Unix timestamp of the price
        inbound_id: ID of the inbound message being responded to

    Returns:
        Message ID from message_log
    """
    price_fixed = int(price * 10**8)

    payload = json.dumps(
        {
            "request_id": request_id,
            "asset": asset,
            "price": price_fixed,
            "timestamp": timestamp,
        }
    )

    idempotency_key = f"price_response:{request_id}"

    cursor = conn.execute(
        """
        INSERT INTO message_log (message_type, payload, idempotency_key, trigger_event, trigger_id)
        VALUES ('price_response', ?, ?, 'price_request', ?)
        ON CONFLICT(idempotency_key) DO NOTHING
        """,
        (payload, idempotency_key, request_id),
    )
    conn.commit()

    message_id = cursor.lastrowid if cursor.rowcount > 0 else 0

    if message_id > 0:
        # Mark the inbound message as processed and link the response
        conn.execute(
            """
            UPDATE inbound_message_log
            SET processed = 1, processed_at = unixepoch(), response_message_id = ?
            WHERE id = ?
            """,
            (message_id, inbound_id),
        )
        conn.commit()
        logger.info(f"Created price response for request {request_id[:16]}...")

        # Submit a withdrawal request to the sequencer for bridge execution
        # This creates a Withdrawal message that validators will sign
        withdrawal_seq = submit_withdrawal_request(
            request_id=request_id,
            asset=asset,
            price=price_fixed,
            timestamp=timestamp,
        )
        if withdrawal_seq:
            logger.info(f"Submitted withdrawal for bridge: sequence={withdrawal_seq}")

    return message_id


def insert_price_request(
    conn: sqlite3.Connection,
    request: PriceRequest,
    log_index: Optional[int] = None,
) -> int:
    """Insert an inbound price request from the chain monitor.

    Args:
        conn: Database connection
        request: PriceRequest from the blockchain event
        log_index: Log index within the transaction

    Returns:
        Inbound message ID
    """
    payload = json.dumps(
        {
            "asset": request.asset,
            "requester": request.requester,
            "max_age": request.max_age,
        }
    )

    cursor = conn.execute(
        """
        INSERT INTO inbound_message_log
            (message_id, message_type, payload, block_number, tx_hash, log_index)
        VALUES (?, 'price_request', ?, ?, ?, ?)
        ON CONFLICT(message_id) DO NOTHING
        """,
        (
            request.request_id,
            payload,
            request.block_number,
            request.tx_hash,
            log_index,
        ),
    )
    conn.commit()

    if cursor.rowcount > 0:
        logger.info(
            f"Recorded price request for {request.asset} from {request.requester[:10]}..."
        )
        return cursor.lastrowid
    else:
        logger.debug(f"Duplicate price request {request.request_id[:16]}..., skipped")
        return 0


def process_pending_price_requests(
    conn: sqlite3.Connection,
    get_price_fn,
) -> int:
    """Process all pending price requests and create response messages.

    Args:
        conn: Database connection
        get_price_fn: Function(asset) -> (price, timestamp) to get current price

    Returns:
        Number of requests processed
    """
    from .schema import get_unprocessed_inbound_messages

    pending = get_unprocessed_inbound_messages(conn)
    processed = 0

    for msg in pending:
        if msg["message_type"] != "price_request":
            continue

        try:
            payload = json.loads(msg["payload"])
            asset = payload["asset"]

            # Get the current price
            price_data = get_price_fn(asset)
            if price_data is None:
                logger.warning(f"No price available for {asset}")
                continue

            price, timestamp = price_data

            # Check max_age if specified
            max_age = payload.get("max_age", 0)
            if max_age > 0:
                age = int(time.time()) - timestamp
                if age > max_age:
                    logger.warning(
                        f"Price for {asset} is too old ({age}s > {max_age}s)"
                    )
                    continue

            # Create response message
            create_price_response_message(
                conn=conn,
                request_id=msg["message_id"],
                asset=asset,
                price=price,
                timestamp=timestamp,
                inbound_id=msg["id"],
            )
            processed += 1

        except Exception as e:
            logger.error(f"Error processing price request: {e}")

    return processed


def get_outbound_message_stats(conn: sqlite3.Connection) -> dict:
    """Get statistics about outbound messages.

    Returns:
        Dict with counts by status
    """
    cursor = conn.execute(
        """
        SELECT status, COUNT(*) as count
        FROM message_log
        GROUP BY status
        """
    )
    stats = {row[0]: row[1] for row in cursor.fetchall()}
    return {
        "pending": stats.get("pending", 0),
        "submitted": stats.get("submitted", 0),
        "confirmed": stats.get("confirmed", 0),
        "failed": stats.get("failed", 0),
        "total": sum(stats.values()),
    }


def get_inbound_message_stats(conn: sqlite3.Connection) -> dict:
    """Get statistics about inbound messages.

    Returns:
        Dict with counts by processed status
    """
    cursor = conn.execute(
        """
        SELECT processed, COUNT(*) as count
        FROM inbound_message_log
        GROUP BY processed
        """
    )
    stats = {row[0]: row[1] for row in cursor.fetchall()}
    return {
        "pending": stats.get(0, 0),
        "processed": stats.get(1, 0),
        "total": sum(stats.values()),
    }
