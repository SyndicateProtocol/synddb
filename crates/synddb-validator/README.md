# synddb-validator

State reconstruction service for SyndDB. Fetches signed messages from the sequencer or storage layer, verifies signatures, and applies changesets to reconstruct the application state.

## Architecture

```
             Sequencer / Storage Layer
                     │
                     │ HTTP GET / GCS fetch
                     ▼
┌─────────────────────────────────────────┐
│   synddb-validator                      │
│                                         │
│  ┌─────────────────┐                    │
│  │    Fetcher      │  (HTTP / GCS)      │
│  └────────┬────────┘                    │
│           │                             │
│  ┌────────▼────────┐                    │
│  │    Verifier     │  (signature check) │
│  └────────┬────────┘                    │
│           │                             │
│  ┌────────▼────────┐                    │
│  │    Applier      │  (SQLite replay)   │
│  └────────┬────────┘                    │
│           │                             │
│  ┌────────▼────────┐                    │
│  │  State Store    │  (sync progress)   │
│  └─────────────────┘                    │
└─────────────────────────────────────────┘
```

## Usage

### Basic Usage

```bash
# Run with HTTP fetcher (fetches from sequencer directly)
# SEQUENCER_PUBKEY is the 64-byte uncompressed secp256k1 public key (128 hex chars)
SEQUENCER_PUBKEY=8318535b54105d4a7aae60c08fc45f9687181b4fdfc625bd1a753fa7397fed753547f11ca8696646f2f3acb08e31016afac23e630c5d11f59f61fef57b0d2aa5 \
SEQUENCER_URL=http://localhost:8433 \
cargo run -p synddb-validator

# Run with GCS fetcher (select via FETCHER_TYPE)
SEQUENCER_PUBKEY=8318535b54105d4a7aae60c08fc45f9687181b4fdfc625bd1a753fa7397fed753547f11ca8696646f2f3acb08e31016afac23e630c5d11f59f61fef57b0d2aa5 \
FETCHER_TYPE=gcs \
GCS_BUCKET=my-bucket \
cargo run -p synddb-validator

# With JSON logging
RUST_LOG=debug LOG_JSON=true cargo run -p synddb-validator
```

### Environment Variables

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `SEQUENCER_PUBKEY` | Yes | - | Expected sequencer public key for signature verification (64-byte uncompressed secp256k1, 128 hex chars) |
| `DATABASE_PATH` | No | `/data/validator.db` | SQLite database for replicated state |
| `STATE_DB_PATH` | No | `/data/validator_state.db` | SQLite database for validator state (sequences) |
| `FETCHER_TYPE` | No | `http` | Fetcher type: `http` or `gcs` |
| `SEQUENCER_URL` | Conditional | - | Sequencer URL (required when `FETCHER_TYPE=http`) |
| `GCS_BUCKET` | Conditional | - | GCS bucket (required when `FETCHER_TYPE=gcs`) |
| `GCS_PREFIX` | No | `sequencer` | Path prefix within GCS bucket |
| `BIND_ADDRESS` | No | `0.0.0.0:8080` | HTTP server bind address |
| `SYNC_INTERVAL` | No | `1s` | Poll interval for new messages |
| `START_SEQUENCE` | No | `0` | Starting sequence number |
| `LOG_JSON` | No | `false` | Enable JSON-formatted logs |
| `SHUTDOWN_TIMEOUT` | No | `30s` | Graceful shutdown timeout |

### Gap Detection

| Variable | Default | Description |
|----------|---------|-------------|
| `GAP_RETRY_COUNT` | `5` | Retries before declaring a sequence gap |
| `GAP_RETRY_DELAY` | `5s` | Delay between gap retry attempts |
| `GAP_SKIP_ON_FAILURE` | `false` | Skip gaps after max retries (use with caution) |

### Batch Sync

When enabled, the validator fetches messages in batches for more efficient synchronization.

| Variable | Default | Description |
|----------|---------|-------------|
| `BATCH_SYNC_ENABLED` | `true` | Enable batch sync when fetcher supports it |
| `BATCH_INDEX_REFRESH_INTERVAL` | `10s` | How often to refresh the batch index |

Both HTTP and GCS fetchers support batch operations. Batch sync automatically falls back to single-message fetching if:
- Batch sync is disabled via configuration
- The batch index is empty (no batches published yet)

### Bridge Signer Mode

The validator can optionally sign withdrawal attestations for the bridge contract.

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `BRIDGE_SIGNER` | No | `false` | Enable bridge signer mode |
| `BRIDGE_CONTRACT` | Conditional | - | Bridge contract address |
| `BRIDGE_CHAIN_ID` | Conditional | - | Chain ID for the bridge contract |
| `BRIDGE_SIGNING_KEY` | Conditional | - | Hex-encoded private key for signing |
| `BRIDGE_SIGNATURE_ENDPOINT` | No | `0.0.0.0:8081` | Endpoint for relayers to fetch signatures |

## API Endpoints

### Health & Status

- `GET /health` - Liveness check (always returns 200)
- `GET /healthz` - Alias for `/health`
- `GET /ready` - Readiness check (verifies sync is running)
- `GET /status` - Validator status (last synced sequence, sync time)

### Status Response

```json
{
  "last_sequence": 1234,
  "synced_at": "2024-01-01T00:00:00Z"
}
```

## Sync Modes

### Single-Message Sync

Fallback mode that fetches messages one at a time (used when batch sync is disabled or no batches are available):

1. Fetch message at next expected sequence
2. Verify sequencer signature
3. Apply changeset to SQLite
4. Record sync progress
5. Repeat

### Batch Sync

Both HTTP and GCS fetchers support batch sync for more efficient synchronization:

1. Build a batch index by listing all available batches
2. Find the batch containing the next expected sequence
3. Fetch and process all messages in the batch
4. Periodically refresh the batch index for new batches
5. Automatically handles gaps between batches

Batch sync reduces the number of network requests when catching up from behind, fetching many messages in a single request instead of one at a time.

## Modules

| Module | Description |
|--------|-------------|
| `config` | Configuration from environment variables |
| `http` | Axum HTTP server for health/status endpoints |
| `validator` | Core sync loop and message processing |
| `sync/fetcher` | Storage fetcher trait and batch support |
| `sync/providers` | Fetcher implementations (HTTP, GCS, mock) |
| `sync/batch_index` | Batch index for efficient sequential sync |
| `sync/verifier` | Signature verification |
| `apply` | Changeset application to SQLite |
| `state` | Sync state persistence |
| `bridge` | Bridge signer mode (optional) |

## Development

```bash
# Run tests
cargo test -p synddb-validator

# Run with test configuration
SEQUENCER_PUBKEY=8318535b54105d4a7aae60c08fc45f9687181b4fdfc625bd1a753fa7397fed753547f11ca8696646f2f3acb08e31016afac23e630c5d11f59f61fef57b0d2aa5 \
SEQUENCER_URL=http://localhost:8433 \
DATABASE_PATH=:memory: \
STATE_DB_PATH=:memory: \
cargo run -p synddb-validator

# Build release binary
cargo build -p synddb-validator --release
```

## Storage

The validator maintains two SQLite databases:

1. **Replicated State** (`DATABASE_PATH`): Contains the reconstructed application state from applied changesets
2. **Validator State** (`STATE_DB_PATH`): Tracks sync progress (last sequence, gaps, etc.)

Both can use `:memory:` for testing or ephemeral deployments.
