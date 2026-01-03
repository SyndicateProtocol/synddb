# Proof Service

GPU-accelerated SP1 proof generation service for TEE attestation verification.

This service receives GCP Confidential Space attestation tokens from TEE services (sequencers/validators) and generates SP1 zero-knowledge proofs that can be verified on-chain.

## Architecture

```
TEE Service                    Proof Service                  On-Chain
    │                              │                              │
    │  POST /prove                 │                              │
    │  {jwt_token, audience}       │                              │
    ├─────────────────────────────►│                              │
    │                              │  Fetch JWKS from Google      │
    │                              │  Verify JWT signature        │
    │                              │  Generate SP1 proof (GPU)    │
    │                              │                              │
    │  {public_values, proof}      │                              │
    │◄─────────────────────────────┤                              │
    │                              │                              │
    │  addKey(public_values, proof)│                              │
    ├─────────────────────────────────────────────────────────────►│
    │                              │                              │
```

## API

### POST /prove

Generate a proof for an attestation token.

**Request:**
```json
{
  "jwt_token": "eyJ...",
  "expected_audience": "https://..."
}
```

**Response (200):**
```json
{
  "public_values": "0x...",
  "proof_bytes": "0x...",
  "tee_address": "0x..."
}
```

**Response (503):** Prover is busy generating another proof.

### GET /health

Health check endpoint.

**Response:**
```json
{
  "status": "ready",
  "prover_busy": false
}
```

## Configuration

| Environment Variable | Default | Description |
|---------------------|---------|-------------|
| `BIND_ADDRESS` | `0.0.0.0:8080` | HTTP server listen address |
| `LOG_JSON` | `false` | Enable JSON log output |
| `SP1_PROVER` | `local` | Prover backend: `local`, `cuda`, or `network` |
| `GOOGLE_OIDC_DISCOVERY_URL` | GCP default | OIDC discovery endpoint for JWKS |
| `JWKS_CACHE_TTL_SECS` | `3600` | How long to cache Google's public keys |

### SP1 Prover Modes

- `local` - CPU-based proving (slow, ~10-30 min per proof)
- `cuda` - GPU-accelerated proving (fast, ~2-5 min per proof)
- `network` - SP1 Network Prover (requires `SP1_PRIVATE_KEY`)

## Deployment

### Local Development

```bash
# Build
cargo build --release

# Run with CPU prover
SP1_PROVER=local ./target/release/proof-service

# Run with GPU prover (requires CUDA)
SP1_PROVER=cuda ./target/release/proof-service
```

### Docker

```bash
# Build image
docker build -t proof-service .

# Run with GPU
docker run --gpus all -p 8080:8080 proof-service
```

### Cloud Run (GPU)

```bash
# Build and push
gcloud builds submit --tag gcr.io/$PROJECT_ID/proof-service

# Deploy with GPU
gcloud run services replace deploy/cloud-run.yaml --region=us-central1
```

The `deploy/cloud-run.yaml` configures:
- NVIDIA L4 GPU
- 8 vCPUs, 32GB RAM
- 15-minute timeout for proof generation
- Single instance (proofs are resource-intensive)
- Container concurrency of 1

## Security Considerations

1. **Rate Limiting**: Each proof takes 2-10 minutes and significant GPU resources. Consider rate limiting or authentication.

2. **Network Security**: The service should only be accessible from TEE workloads. Consider VPC Service Controls or mTLS.

3. **Attestation Freshness**: JWT tokens expire after 1 hour. The proof embeds the validity window.

## Development

### Prerequisites

- Rust 1.75+
- SP1 toolchain: `curl -L https://sp1.succinct.xyz | bash && sp1up`
- CUDA toolkit (for GPU proving)

### Building the SP1 Program

The SP1 program is built automatically via `build.rs`:

```bash
# The program is at crates/synddb-bootstrap/sp1/program/
cargo build -p proof-service
```

### Testing

```bash
# Run with mock prover (no real proof generation)
SP1_PROVER=mock cargo run
```
