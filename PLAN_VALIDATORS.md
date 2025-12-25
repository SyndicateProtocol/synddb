# Validator Implementation Plan

> Implementation details for Message Passing Bridge validators.
> See `SPEC_MESSAGE_BRIDGE.md` for the specification.

## Overview

This document contains implementation details, code patterns, and examples for Primary and Witness validators.

## 1. Validator Architecture

```
validator/
├── main.rs                    # Entry point, config loading
├── config.rs                  # Configuration structs
├── http/
│   ├── server.rs              # Axum HTTP server
│   ├── handlers.rs            # Request handlers
│   └── auth.rs                # mTLS/API key authentication
├── validation/
│   ├── pipeline.rs            # Validation pipeline orchestration
│   ├── schema.rs              # JSON Schema validation
│   ├── invariants/
│   │   ├── mod.rs
│   │   ├── on_chain.rs        # On-chain state invariants
│   │   ├── oracle.rs          # Oracle price invariants
│   │   └── app_logic.rs       # Application logic invariants
│   └── nonce.rs               # Nonce tracking
├── signing/
│   ├── eip712.rs              # EIP-712 typed data signing
│   └── tee.rs                 # TEE key management
├── storage/
│   ├── publisher.rs           # Storage layer publication
│   ├── arweave.rs
│   ├── ipfs.rs
│   └── gcs.rs
├── bridge/
│   ├── client.rs              # Bridge contract interaction
│   └── events.rs              # Event monitoring
└── witness/
    ├── discovery.rs           # Message discovery from storage
    └── rederivation.rs        # Metadata re-derivation
```

## 2. Validation Pipeline

### 2.1 Pipeline Stages

```python
def validate_message(message: Message) -> ValidationResult:
    """Primary Validator validation pipeline"""

    # 2.1 REPLAY PROTECTION
    message_id = compute_message_id(message)
    if is_processed(message_id):
        raise ValidationError("REPLAY_DETECTED")

    # 2.2 NONCE CHECK
    expected_nonce = get_last_nonce(message.domain) + 1
    if message.nonce != expected_nonce:
        raise ValidationError("INVALID_NONCE", f"expected {expected_nonce}")

    # 2.3 FRESHNESS CHECK
    if abs(message.timestamp - time.now()) > MAX_CLOCK_DRIFT:
        raise ValidationError("TIMESTAMP_EXPIRED")

    # 2.4 APPLICATION AUTHORIZATION
    if not is_app_authorized(message.domain):
        raise ValidationError("APP_NOT_AUTHORIZED")

    # 2.5 MESSAGE TYPE VALIDATION
    config = get_message_type_config(message.messageType)
    if not config or not config.enabled:
        raise ValidationError("MESSAGE_TYPE_NOT_REGISTERED")

    # 2.6 CALLDATA VALIDATION
    if not validate_calldata(message.calldata, message.messageType):
        raise ValidationError("CALLDATA_INVALID")

    # 2.7 SCHEMA VALIDATION
    schema = fetch_schema(config.schemaUri)
    if not validate_json_schema(message.metadata, schema):
        raise ValidationError("SCHEMA_VALIDATION_FAILED")

    # 2.8 INVARIANT CHECKS
    check_invariants(message)

    # 2.9 CUSTOM RULES
    check_rate_limits(message)
    check_amount_thresholds(message)

    return ValidationResult.success(message_id)
```

### 2.2 Nonce Tracking

```python
class NonceTracker:
    def __init__(self):
        self.last_nonce = {}  # domain -> last consumed nonce

    def validate_nonce(self, domain: str, nonce: int) -> bool:
        expected = self.last_nonce.get(domain, 0) + 1
        return nonce == expected

    def consume_nonce(self, domain: str, nonce: int):
        """Called when message is initialized OR rejected on Bridge"""
        self.last_nonce[domain] = nonce

    def on_initialize(self, domain: str, nonce: int):
        """Message accepted and initialized"""
        self.consume_nonce(domain, nonce)

    def on_reject(self, domain: str, nonce: int):
        """Message rejected via rejectProposal()"""
        self.consume_nonce(domain, nonce)
```

## 3. Invariant Checking

### 3.1 On-Chain State Invariants

```python
def check_supply_cap_invariant(message: Message, rpc_client: RpcClient):
    """Check that minting won't exceed supply cap"""

    # 1. Extract mint amount from calldata
    _, recipient, amount = decode_calldata(message.calldata)

    # 2. Query current supply from target contract
    target_contract = get_target_contract(message.messageType)
    current_supply = rpc_client.call(target_contract, "totalSupply()")

    # 3. Get max supply from metadata
    max_supply = int(message.metadata["maxSupply"])

    # 4. Check invariant: current + mint <= max
    if current_supply + amount > max_supply:
        raise InvariantViolation(
            "INVARIANT_VIOLATED",
            f"Supply cap exceeded: {current_supply} + {amount} > {max_supply}"
        )

    return True
```

### 3.2 Oracle Price Invariants

```python
def check_price_deviation_invariant(message: Message, oracle_client: OracleClient):
    """Check that price is within acceptable deviation from oracle"""

    # 1. Get application's claimed rate
    app_rate = Decimal(message.metadata["exchangeRate"])
    max_deviation_pct = Decimal(message.metadata.get("maxDeviation", "5"))

    # 2. Fetch oracle price
    from_token = message.metadata["fromToken"]
    to_token = message.metadata["toToken"]

    oracle_price = oracle_client.get_price(
        feed=f"{from_token}/{to_token}",
        provider="chainlink"
    )

    # 3. Check freshness
    if oracle_price.timestamp < time.now() - MAX_ORACLE_AGE:
        raise InvariantViolation("INVARIANT_DATA_STALE", "Oracle price is stale")

    # 4. Calculate deviation
    deviation_pct = abs(app_rate - oracle_price.value) / oracle_price.value * 100

    # 5. Check threshold
    if deviation_pct > max_deviation_pct:
        raise InvariantViolation(
            "INVARIANT_VIOLATED",
            f"Price deviation {deviation_pct:.2f}% exceeds max {max_deviation_pct}%"
        )

    return True
```

### 3.3 Application Logic Invariants

```python
def check_game_invariants(message: Message):
    """Check game-specific business rules"""
    metadata = message.metadata

    # Invariant 1: Only one winner per game
    if metadata["gameWinnerCount"] != 0:
        raise InvariantViolation(
            "INVARIANT_VIOLATED",
            f"Game {metadata['gameId']} already has a winner"
        )

    # Invariant 2: Minimum play interval
    time_since_last_play = (
        metadata["currentTimestamp"] - metadata["playerLastPlayTimestamp"]
    )
    if time_since_last_play < metadata["minPlayInterval"]:
        raise InvariantViolation(
            "INVARIANT_VIOLATED",
            f"Player must wait {metadata['minPlayInterval'] - time_since_last_play}s"
        )

    return True
```

### 3.4 Verification API Client

```python
def verify_via_api(message: Message) -> bool:
    """Call application's Verification API for Witness validation"""
    api_url = message.metadata.get("verificationApiUrl")

    if not api_url:
        # No verification API - trust Primary
        log.info("No verificationApiUrl, trusting Primary validation")
        return True

    try:
        response = http_client.post(
            f"{api_url}/verify",
            json={
                "messageId": message.id,
                "metadata": message.metadata
            },
            timeout=5
        )

        result = response.json()
        if not result["valid"]:
            raise InvariantViolation(
                "INVARIANT_VERIFICATION_FAILED",
                result.get("reason", "Verification API rejected")
            )

        return True

    except Timeout:
        # Configurable: reject or trust Primary on timeout
        if config.reject_on_verification_timeout:
            raise InvariantViolation("INVARIANT_DATA_UNAVAILABLE", "Verification API timeout")
        return True
```

## 4. Witness Validator

### 4.1 Message Discovery

```python
class WitnessDiscovery:
    def __init__(self, bridge_client, storage_clients):
        self.bridge = bridge_client
        self.storage = storage_clients

    async def watch_for_messages(self):
        """Watch for MessageInitialized events on Bridge"""
        async for event in self.bridge.watch_events("MessageInitialized"):
            message_id = event.messageId
            storage_ref = event.storageRef

            # Fetch message from storage
            message = await self.fetch_from_storage(storage_ref)

            # Verify content matches event
            if compute_message_id(message) != message_id:
                log.error(f"Message ID mismatch for {message_id}")
                continue

            yield message

    async def fetch_from_storage(self, storage_ref: str) -> Message:
        """Fetch message from storage layer (with fallback)"""
        uris = storage_ref.split("|")

        for uri in uris:
            try:
                if uri.startswith("ar://"):
                    return await self.storage.arweave.fetch(uri)
                elif uri.startswith("ipfs://"):
                    return await self.storage.ipfs.fetch(uri)
                elif uri.startswith("gcs://"):
                    return await self.storage.gcs.fetch(uri)
            except Exception as e:
                log.warning(f"Failed to fetch from {uri}: {e}")
                continue

        raise StorageError("Failed to fetch from all storage sources")
```

### 4.2 Metadata Re-Derivation

```python
class MetadataRederivation:
    def __init__(self, rpc_clients, oracle_client):
        self.rpc = rpc_clients
        self.oracle = oracle_client

    async def rederive_and_verify(self, message: Message):
        """Re-derive verifiable metadata fields"""
        metadata = message.metadata

        # Source transaction verification
        if "sourceTxHash" in metadata:
            tx = await self.rpc[metadata["sourceChain"]].get_transaction(
                metadata["sourceTxHash"]
            )
            if not tx:
                raise RederivationError("Source transaction not found")

            # Verify deposit amount from event logs
            if "depositAmount" in metadata:
                actual_amount = self.extract_deposit_amount(tx)
                if actual_amount != int(metadata["depositAmount"]):
                    raise RederivationError(
                        f"Deposit amount mismatch: {actual_amount} != {metadata['depositAmount']}"
                    )

        # Price verification
        if "exchangeRate" in metadata:
            oracle_price = await self.oracle.get_price(
                metadata.get("fromToken"),
                metadata.get("toToken")
            )
            app_rate = Decimal(metadata["exchangeRate"])
            deviation = abs(app_rate - oracle_price) / oracle_price

            if deviation > Decimal("0.05"):  # 5% threshold
                raise RederivationError(
                    f"Price deviation too large: {deviation:.2%}"
                )

        # On-chain state verification
        if "currentTotalSupply" in metadata:
            actual_supply = await self.rpc.target.call("totalSupply()")
            # Allow small differences due to timing
            reported = int(metadata["currentTotalSupply"])
            if abs(actual_supply - reported) > reported * 0.01:  # 1% tolerance
                raise RederivationError(
                    f"Supply mismatch: {actual_supply} vs reported {reported}"
                )
```

## 5. HTTP API

### 5.1 Endpoints

```yaml
POST /messages
  Description: Submit message for validation and signing
  Auth: mTLS or API key
  Request:
    messageType: string
    calldata: bytes
    metadata: object
    nonce: uint64
    timestamp: uint64
    domain: bytes32
    value?: uint256
  Response:
    status: "accepted" | "rejected"
    messageId?: bytes32
    signature?: bytes
    storageRef?: string
    error?: { code, message, details }

GET /messages/{messageId}
  Description: Get message status
  Response:
    id: bytes32
    status: "pending" | "signed" | "published" | "submitted" | "executed"
    signatures: address[]
    storageRef?: string
    bridgeTxHash?: bytes32

GET /health
  Response:
    healthy: boolean
    synced: boolean
    bridgeConnection: boolean
    storageConnection: boolean
    lastProcessedNonce: map[domain => nonce]

GET /schemas/{messageType}
  Response:
    schema: object
    hash: bytes32
    source: "chain" | "ipfs" | "arweave"
    cached: boolean
```

### 5.2 Error Response Format

```json
{
  "status": "rejected",
  "error": {
    "code": "INVARIANT_VIOLATED",
    "message": "Supply cap would be exceeded",
    "details": {
      "currentSupply": "9500000000000000000000",
      "mintAmount": "1000000000000000000000",
      "maxSupply": "10000000000000000000000"
    }
  }
}
```

## 6. Storage Publication

### 6.1 Publication Flow

```python
class StoragePublisher:
    def __init__(self, config):
        self.arweave = ArweaveClient(config.arweave) if config.arweave else None
        self.ipfs = IpfsClient(config.ipfs) if config.ipfs else None
        self.gcs = GcsClient(config.gcs) if config.gcs else None

    async def publish(self, message: Message, signature: bytes) -> str:
        """Publish message to storage layer(s), return storageRef"""
        record = StorageRecord(
            message=message,
            primarySignature=SignatureRecord(
                validator=self.address,
                signature=signature,
                signedAt=int(time.time())
            ),
            publication=PublicationRecord(
                publishedBy=self.address,
                publishedAt=int(time.time())
            )
        )

        refs = []

        # Publish to configured storage layers
        if self.arweave:
            tx_id = await self.arweave.publish(record)
            refs.append(f"ar://{tx_id}")

        if self.ipfs:
            cid = await self.ipfs.publish(record)
            refs.append(f"ipfs://{cid}")

        if self.gcs:
            path = await self.gcs.publish(record)
            refs.append(f"gcs://{path}")

        return "|".join(refs)
```

## 7. EIP-712 Signing

```python
class EIP712Signer:
    DOMAIN = {
        "name": "SyndBridge",
        "version": "1",
        "chainId": None,  # Set at runtime
        "verifyingContract": None  # Set at runtime
    }

    MESSAGE_TYPE = {
        "Message": [
            {"name": "messageId", "type": "bytes32"},
            {"name": "messageType", "type": "string"},
            {"name": "calldata", "type": "bytes"},
            {"name": "metadataHash", "type": "bytes32"},
            {"name": "nonce", "type": "uint64"},
            {"name": "timestamp", "type": "uint64"},
            {"name": "domain", "type": "bytes32"},
        ]
    }

    def sign(self, message: Message, private_key: bytes) -> bytes:
        struct_hash = keccak256(encode_abi(
            ["bytes32", "bytes32", "bytes32", "bytes32", "uint64", "uint64", "bytes32"],
            [
                self.MESSAGE_TYPEHASH,
                message.id,
                keccak256(message.messageType.encode()),
                keccak256(message.calldata),
                keccak256(canonicalize(message.metadata)),
                message.nonce,
                message.timestamp,
                message.domain
            ]
        ))

        digest = keccak256(b"\x19\x01" + self.domain_separator + struct_hash)
        return sign_digest(digest, private_key)
```

## 8. Configuration

```toml
[validator]
mode = "primary"  # or "witness"
address = "0x..."
private_key_path = "/secrets/signing_key"  # or use TEE

[bridge]
address = "0x..."
rpc_url = "https://..."
chain_id = 1

[storage]
[storage.arweave]
gateway = "https://arweave.net"
wallet_path = "/secrets/arweave_wallet.json"

[storage.ipfs]
api_url = "http://localhost:5001"
pin = true

[http]
bind = "0.0.0.0:8080"
tls_cert = "/certs/server.crt"
tls_key = "/certs/server.key"
client_ca = "/certs/ca.crt"  # For mTLS

[oracle]
chainlink_rpc = "https://..."
price_feeds = ["ETH/USD", "BTC/USD"]
max_age_seconds = 300

[validation]
max_clock_drift_seconds = 60
reject_on_verification_timeout = false
```

## 9. Implementation Checklist

### Primary Validator
- [ ] HTTP server with mTLS
- [ ] Validation pipeline
- [ ] Schema fetching and caching
- [ ] Invariant checking (on-chain, oracle, app logic)
- [ ] Nonce tracking
- [ ] EIP-712 signing
- [ ] Storage publication (Arweave, IPFS, GCS)
- [ ] Bridge interaction (initialize, sign)
- [ ] Health/status endpoints

### Witness Validator
- [ ] Event monitoring (MessageInitialized)
- [ ] Storage fetching
- [ ] Metadata re-derivation
- [ ] Verification API client
- [ ] Independent invariant checking
- [ ] Bridge interaction (sign only)

### Shared
- [ ] Configuration loading
- [ ] TEE key management
- [ ] Oracle client (Chainlink, Pyth)
- [ ] RPC client (multi-chain)
- [ ] Logging and metrics
- [ ] Error handling
