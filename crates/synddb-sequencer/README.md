# synddb-sequencer

Message ordering and signing service for SyndDB. Receives messages from `synddb-client` applications, assigns monotonic sequence numbers, signs them with `secp256k1`, and optionally persists to storage backends.

## Architecture

```
synddb-client (App TEE)
       │
       │ HTTP POST /changesets, /withdrawals
       ▼
┌─────────────────────────┐
│   synddb-sequencer      │
│   (Sequencer TEE)       │
│                         │
│  ┌─────────────────┐    │
│  │   HTTP API      │    │
│  └────────┬────────┘    │
│           │             │
│  ┌────────▼────────┐    │
│  │     Inbox       │    │
│  │  (Sequencing)   │    │
│  └────────┬────────┘    │
│           │             │
│  ┌────────▼────────┐    │
│  │    Signer       │    │
│  │  (secp256k1)    │    │
│  └────────┬────────┘    │
│           │             │
│  ┌────────▼────────┐    │
│  │   Publisher     │    │
│  │  (GCS, etc.)    │    │
│  └─────────────────┘    │
└─────────────────────────┘
```

## Usage

### Basic Usage

```bash
# Run with a signing key (required)
SIGNING_KEY=<hex-private-key> cargo run -p synddb-sequencer

# With GCS persistence (select via PUBLISHER_TYPE)
SIGNING_KEY=<key> PUBLISHER_TYPE=gcs GCS_BUCKET=my-bucket cargo run -p synddb-sequencer

# With JSON logging
SIGNING_KEY=<key> RUST_LOG=debug RUST_LOG_JSON=true cargo run -p synddb-sequencer
```

### Environment Variables

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `SIGNING_KEY` | Yes | - | Hex-encoded secp256k1 private key |
| `BIND_ADDRESS` | No | `0.0.0.0:8433` | HTTP server bind address |
| `PUBLISHER_TYPE` | No | `local` | Transport type: `local` (SQLite) or `gcs` |
| `GCS_BUCKET` | No | - | GCS bucket for persistence (when `PUBLISHER_TYPE=gcs`) |
| `GCS_PREFIX` | No | `sequencer` | Path prefix within GCS bucket |
| `REQUEST_TIMEOUT` | No | `30s` | Request timeout for HTTP operations |
| `MAX_MESSAGE_SIZE` | No | `10485760` | Maximum message size in bytes (10MB) |
| `VERIFY_ATTESTATION` | No | `false` | Enable TEE attestation verification |
| `ATTESTATION_SERVICE_URL` | No | - | Attestation service URL for TEE token verification |
| `RUST_LOG` | No | `info` | Log level/filter (e.g., `debug`, `synddb_sequencer=trace`) |
| `RUST_LOG_JSON` | No | `false` | Enable JSON-formatted logs |
| `SHUTDOWN_TIMEOUT` | No | `30s` | Graceful shutdown timeout |

## API Endpoints

### Health & Status

- `GET /health` - Liveness check (always returns 200)
- `GET /ready` - Readiness check (verifies publisher connectivity)
- `GET /status` - Sequencer status (current sequence, signer public key)

### Message Submission

- `POST /changesets` - Submit a changeset for sequencing
- `POST /withdrawals` - Submit a withdrawal request for sequencing
- `GET /messages/:sequence` - Retrieve a signed message by sequence number

### Request/Response Examples

```bash
# Submit a changeset
curl -X POST http://localhost:8080/changesets \
  -H "Content-Type: application/json" \
  -d '{"data": "base64-encoded-changeset"}'

# Response
{
  "sequence": 1,
  "signature": "0x...",
  "signer": "0x...",
  "timestamp": 1234567890,
  "data_hash": "0x..."
}

# Submit a withdrawal
curl -X POST http://localhost:8080/withdrawals \
  -H "Content-Type: application/json" \
  -d '{
    "request_id": "unique-id",
    "recipient": "0x1234567890123456789012345678901234567890",
    "amount": "1000000000000000000",
    "token": "0x..."
  }'
```

## Modules

| Module | Description |
|--------|-------------|
| `config` | Configuration from environment variables |
| `http_api` | Axum HTTP server and route handlers |
| `inbox` | Message sequencing with atomic counter |
| `signer` | secp256k1 signing via alloy |
| `attestation` | TEE attestation token verification (GCP Confidential Space) |
| `publish` | Pluggable storage backends (GCS, mock) |

## Transports

Storage transports are selected at runtime via `PUBLISHER_TYPE`:

| Transport | Description |
|-----------|-------------|
| `local` (default) | SQLite-based local storage |
| `gcs` | Google Cloud Storage |

Future transports (Celestia, EigenDA, Arweave) will be added as runtime options.

## Development

```bash
# Run tests
cargo test -p synddb-sequencer

# Run with test key (DO NOT use in production)
SIGNING_KEY=ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80 \
  cargo run -p synddb-sequencer

# Build release binary
cargo build -p synddb-sequencer --release
```

## Storage Layout

Messages are published to GCS as atomic batches:

```
gs://{bucket}/{prefix}/
└── batches/
    ├── 000000000001_000000000050.json   # messages 1-50
    ├── 000000000051_000000000100.json   # messages 51-100
    └── ...
```

**Batch filename format**: `{start:012}_{end:012}.json`
- `start` - First sequence number in the batch (inclusive), zero-padded to 12 digits
- `end` - Last sequence number in the batch (inclusive), zero-padded to 12 digits

**Benefits**:
- Atomic publication of messages with state (no separate state file)
- State is implicit (highest `end` sequence across all batches)
- Files sort lexicographically in sequence order
- Supports ~1 trillion sequences (12 digits)

**State Recovery**: On startup, the sequencer scans batch filenames to find the highest `end` sequence number.

## Security

- The `SIGNING_KEY` must be kept secret - it signs all sequenced messages
- In production, run in a TEE (GCP Confidential Space) with attestation enabled
- The sequencer verifies client attestation tokens when `tee` feature is enabled
