# GCP Deployment Plan

First end-to-end deployment on Google Cloud for testing purposes.

## Deployment Readiness Summary

| Area | Status | Notes |
|------|--------|-------|
| Dockerization | Ready | Multi-stage builds, all targets, tested |
| Health checks | Ready | `/health`, `/ready`, graceful shutdown |
| Logging | Ready | Structured JSON, Cloud Trace correlation |
| GCS integration | Ready | Publisher + fetcher complete, E2E tested |
| TEE attestation | Ready | Flow documented, known limitations acceptable |
| Metrics | Partial | Infrastructure ready, no app metrics defined |
| Tracing | Partial | Available via `otel` feature flag |
| Configuration validation | Missing | Services don't fail fast on missing config |
| Operational docs | Missing | No runbooks or recovery procedures |

## Ready Components

### Docker

Multi-stage Dockerfile in `docker/Dockerfile` supports all service targets:
- sequencer
- validator
- customer-app
- e2e
- stress-test

Uses Debian slim base images, optimized release builds, ca-certificates included.

### Health Checks

**Sequencer:**
- `GET /health` - Liveness (always 200)
- `GET /ready` - Readiness (checks publisher)
- `GET /status` - Current sequence and signer pubkey

**Validator:**
- `GET /health` - Liveness
- `GET /ready` - Readiness (verifies sync running)
- `GET /status` - Last synced sequence and timestamp

Graceful shutdown handles SIGTERM with 30s timeout, flushes pending batches.

### Logging

- Structured logging via `tracing` macros
- JSON output: `RUST_LOG_JSON=true`
- Cloud Trace correlation when `GCP_PROJECT_ID` is set
- Request IDs via tower-http middleware

### GCS Integration

- Batch storage: `gs://{bucket}/{prefix}/batches/{start:012}_{end:012}.cbor.zst`
- Supports emulator via `GCS_STORAGE_EMULATOR_HOST`
- E2E tests in `.github/workflows/e2e.yml`

### TEE Attestation

See `docs/analysis/tee-attestation-analysis.md` for detailed status:
- Debug mode verification: Fixed
- JWT signature verification: Fixed
- JWKS key rotation: Manual process (acceptable for testing)

## Gaps to Address

### Before First Deployment

#### 1. Configuration Validation

Services should fail fast with clear errors when production-critical config is missing.

**Validator requires:**
- `SEQUENCER_PUBKEY` - Must be set to verify message signatures
- `GCS_BUCKET` - Required when using GCS fetcher

**Sequencer requires:**
- `GCS_BUCKET` - Required when using GCS publisher

Current behavior: Services may start but fail later with unclear errors.

#### 2. GCS Setup Documentation

Document the following for production setup:
- Bucket creation with recommended settings (versioning, access control)
- Service account creation and IAM permissions
- Workload Identity Federation binding for Confidential Space

### During/After Testing

#### 3. App-Level Metrics

Infrastructure is ready (Prometheus exporter at `/metrics`) but no application metrics are defined.

Recommended metrics:
- Sequencer: messages/sec, sequence number, batch publish latency
- Validator: sync lag, validation errors, last synced sequence

#### 4. Basic Alerting

No alerting rules defined. Recommend alerts for:
- Sequence gaps detected
- Validation failures
- Publisher errors
- Sync lag exceeding threshold

#### 5. Operational Runbooks

Missing documentation:
- Startup/shutdown procedures
- Recovery from sequence gaps
- Database restoration
- Troubleshooting common failures

## Deferred (Post-Testing)

### CI/CD Deployment Workflows

Not needed for manual first deployment:
- Cloud Run deployment automation
- Artifact Registry image push
- Staging environment provisioning
- Canary deployment procedures

### Advanced Observability

- OpenTelemetry distributed tracing (requires `otel` feature)
- Custom Cloud Monitoring dashboards
- Sophisticated alerting rules

### Disaster Recovery

- State restoration testing from GCS batches
- RTO/RPO targets documentation
- Recovery procedure drills

### Load Testing

- GCS latency/throughput under load
- Multi-validator sync performance
- Fault injection testing

## GCS Bucket Setup

### Recommended Configuration

```bash
# Create bucket
gcloud storage buckets create gs://synddb-test \
  --location=us-central1 \
  --uniform-bucket-level-access

# Create service account
gcloud iam service-accounts create synddb-sequencer \
  --display-name="SyndDB Sequencer"

# Grant storage permissions
gcloud storage buckets add-iam-policy-binding gs://synddb-test \
  --member="serviceAccount:synddb-sequencer@PROJECT.iam.gserviceaccount.com" \
  --role="roles/storage.objectAdmin"
```

### Lifecycle Policy (Optional)

For test environments, consider automatic cleanup of old batches:

```json
{
  "lifecycle": {
    "rule": [{
      "action": {"type": "Delete"},
      "condition": {"age": 30}
    }]
  }
}
```

## Environment Variables Reference

### Sequencer

| Variable | Required | Description |
|----------|----------|-------------|
| `GCS_BUCKET` | Yes (for GCS) | Target bucket for batch storage |
| `GCS_PREFIX` | No | Path prefix within bucket (default: `sequencer`) |
| `GCP_PROJECT_ID` | No | Enables Cloud Trace log correlation |
| `RUST_LOG` | No | Log level filter (default: `info`) |
| `RUST_LOG_JSON` | No | Enable JSON log output |
| `BATCH_INTERVAL` | No | Batch flush interval (default: `1s`) |
| `BATCH_SIZE` | No | Max messages per batch (default: `50`) |

### Validator

| Variable | Required | Description |
|----------|----------|-------------|
| `SEQUENCER_PUBKEY` | Yes | Hex-encoded sequencer public key |
| `GCS_BUCKET` | Yes (for GCS) | Source bucket for batch fetching |
| `GCS_PREFIX` | No | Path prefix within bucket |
| `DATABASE_PATH` | No | Replicated state database (default: `:memory:`) |
| `STATE_DB_PATH` | No | Sync progress database (default: `:memory:`) |
| `SYNC_INTERVAL` | No | Polling interval (default: `1s`) |

## TEE Key Bootstrap

Both sequencers and validators generate ephemeral signing keys at startup. When a service restarts with `ENABLE_KEY_BOOTSTRAP=true`:

1. New signing key is generated inside the TEE
2. TEE attestation token is obtained from GCP Confidential Space
3. SP1 proof is generated via the GPU proof service
4. Proof is submitted to `TeeKeyManager` contract, registering the new public key on-chain

This is the intended model - keys are bound to TEE instances and verified via on-chain attestation proofs, not externally managed via secrets.

### Bootstrap Prerequisites

#### 1. Deploy Smart Contracts

The contracts are in `contracts/src/attestation/`:

```bash
# Required environment variables
export SP1_VERIFIER_ADDRESS=<deployed SP1 verifier address>
export ATTESTATION_VERIFIER_VKEY=<from SP1 program compilation>
export EXPECTED_IMAGE_DIGEST_HASH=$(cast keccak "sha256:<your-image-digest>")
export EXPIRATION_TOLERANCE=3600  # 1 hour

# Deploy AttestationVerifier and TeeKeyManager
forge script script/DeployAttestationVerifier.s.sol \
  --rpc-url $RPC_URL \
  --broadcast \
  --verify

# Post-deployment: Add Google's trusted JWK hashes
# Get current JWK kids from: https://confidentialcomputing.googleapis.com/.well-known/openid-configuration
cast send $ATTESTATION_VERIFIER "addTrustedJwkHash(bytes32)" $(cast keccak "<jwk_kid>")
```

#### 2. Deploy Proof Service

The GPU proof service generates SP1 proofs from attestation tokens:

```bash
cd services/proof-service

# Build container
docker build -t gcr.io/$PROJECT_ID/proof-service .

# Push to registry
docker push gcr.io/$PROJECT_ID/proof-service

# Deploy to Cloud Run with GPU
gcloud run services replace deploy/cloud-run.yaml --region=us-central1
```

See `services/proof-service/README.md` for detailed configuration.

#### 3. Build TEE Services with Bootstrap

```bash
# Build with TEE feature enabled
cargo build --release --features tee -p synddb-sequencer
cargo build --release --features tee -p synddb-validator
```

### Bootstrap Configuration

| Variable | Required | Description |
|----------|----------|-------------|
| `ENABLE_KEY_BOOTSTRAP` | Yes | Set to `true` to enable |
| `TEE_KEY_MANAGER_ADDRESS` | Yes | Deployed TeeKeyManager contract |
| `BOOTSTRAP_RPC_URL` | Yes | Ethereum RPC endpoint |
| `BOOTSTRAP_CHAIN_ID` | Yes | Chain ID for transactions |
| `PROOF_SERVICE_URL` | Yes | URL of GPU proof service |
| `ATTESTATION_AUDIENCE` | Yes | Expected audience claim |
| `PROOF_TIMEOUT` | No | Proof generation timeout (default: 10m) |
| `BOOTSTRAP_TIMEOUT` | No | Total bootstrap timeout (default: 15m) |
| `MIN_BOOTSTRAP_BALANCE` | No | Minimum gas balance in wei (default: 0.1 ETH) |

### Bootstrap Flow

```
Service starts in TEE
       │
       ▼
Generate ephemeral key
       │
       ▼
Log TEE address ──────────► Operator funds address with ETH
       │
       ▼
Wait for balance (polls every 5s)
       │
       ▼
Fetch attestation token from /run/container_launcher/teeserver.sock
       │
       ▼
Call proof service (2-10 min with GPU)
       │
       ▼
Submit addKey(publicValues, proof) to TeeKeyManager
       │
       ▼
Wait for tx confirmation
       │
       ▼
Verify isKeyValid(address) == true
       │
       ▼
Bootstrap complete, start main service
```

### Pre-funding TEE Keys

Since TEE keys are ephemeral and pay their own gas:

1. On first boot, service logs: `TEE address: 0x... - fund this address before bootstrap can complete`
2. Operator sends ETH to this address (minimum 0.1 ETH by default)
3. Bootstrap waits until balance is sufficient, then proceeds

For testnet deployments, consider a faucet mechanism or pre-funding a pool of addresses.
