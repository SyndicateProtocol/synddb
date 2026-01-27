# Proof Service

RISC Zero GPU proof generation service for TEE attestation verification.

This service receives GCP Confidential Space attestation tokens from TEE services (sequencers/validators) and generates RISC Zero zero-knowledge proofs that can be verified on-chain. The proofs attest that:

1. The JWT token was signed by Google's JWKS keys
2. The TEE is running the expected container image
3. Secure boot is enabled and debug mode is disabled
4. The image signature is valid (secp256k1 ECDSA for on-chain ecrecover)

## Architecture

```
TEE Service                    Proof Service                  On-Chain
    │                              │                              │
    │  POST /prove                 │                              │
    │  {jwt_token, audience,       │                              │
    │   evm_public_key,            │                              │
    │   image_signature}           │                              │
    ├─────────────────────────────►│                              │
    │                              │  Fetch JWKS from Google      │
    │                              │  Generate RISC Zero proof    │
    │                              │  (local GPU)                 │
    │                              │                              │
    │  {public_values, proof,      │                              │
    │   tee_address}               │                              │
    │◄─────────────────────────────┤                              │
    │                              │                              │
    │  Register key via relayer    │                              │
    │  (public_values, proof)      │                              │
    ├─────────────────────────────────────────────────────────────►│
    │                              │                  Verify proof │
    │                              │        Verify image signature │
    │                              │              Register TEE key │
```

## API

### POST /prove

Generate a proof for an attestation token.

**Request:**
```json
{
  "jwt_token": "eyJ...",
  "expected_audience": "https://...",
  "evm_public_key": "0x...",
  "image_signature": "0x..."
}
```

| Field | Type | Description |
|-------|------|-------------|
| `jwt_token` | string | Raw JWT attestation token from GCP Confidential Space |
| `expected_audience` | string | Expected audience claim in the JWT |
| `evm_public_key` | hex string | 64-byte uncompressed secp256k1 public key (no 0x04 prefix) |
| `image_signature` | hex string | 65-byte secp256k1 ECDSA signature (r \|\| s \|\| v) over keccak256(image_digest) |

**Response (200):**
```json
{
  "public_values": "0x...",
  "proof_bytes": "0x...",
  "tee_address": "0x..."
}
```

| Field | Type | Description |
|-------|------|-------------|
| `public_values` | hex string | ABI-encoded `PublicValuesStruct` (journal) for on-chain verification |
| `proof_bytes` | hex string | RISC Zero Groth16 proof bytes (seal) |
| `tee_address` | address | Ethereum address derived from the EVM public key |

**Response (400):** Permanent error (invalid inputs, expired JWT). Do not retry.

**Response (503):** Transient error (service unavailable). May retry.

### GET /health

Health check endpoint.

**Response:**
```json
{
  "status": "ready"
}
```

### GET /image-id

Get the RISC Zero image ID for contract configuration.

**Response:**
```json
{
  "image_id": "0x..."
}
```

The image ID is a bytes32 value that identifies the RISC Zero program. This is needed when configuring the on-chain verifier contract.

## Public Values Structure

The proof commits to the following values that are verified on-chain:

```solidity
struct PublicValuesStruct {
    bytes32 jwk_key_hash;        // Hash of JWK key ID that signed the token
    uint64 validity_window_start; // Token issued-at timestamp
    uint64 validity_window_end;   // Token expiration timestamp
    bytes32 image_digest_hash;    // Hash of container image digest
    address tee_signing_key;      // Address derived from EVM public key
    bool secboot;                 // Secure boot enabled
    bool dbgstat_disabled;        // Debug mode disabled (production)
    bytes32 audience_hash;        // Hash of audience claim
    bytes32 image_sig_r;          // Image signature R component (secp256k1)
    bytes32 image_sig_s;          // Image signature S component (secp256k1)
    uint8 image_sig_v;            // Image signature V component (secp256k1)
}
```

The on-chain `RiscZeroAttestationVerifier` contract:
1. Verifies the RISC Zero Groth16 proof using the image ID
2. Checks the JWK hash is trusted (Google's signing keys)
3. Validates the timestamp window
4. Requires secure boot and production mode
5. Verifies the image digest matches the expected value
6. Recovers the image signer address using ecrecover
7. Checks the recovered address is in the trusted registry

## Configuration

| Environment Variable | Default | Description |
|---------------------|---------|-------------|
| `BIND_ADDRESS` | `0.0.0.0:8083` | HTTP server listen address |
| `LOG_JSON` | `false` | Enable JSON log output |
| `GOOGLE_OIDC_DISCOVERY_URL` | GCP default | OIDC discovery endpoint for JWKS |
| `JWKS_CACHE_TTL_SECS` | `3600` | How long to cache Google's public keys |

## GPU Support

RISC Zero supports native GPU proving via CUDA. When deployed on Cloud Run with L4 GPUs, proof generation takes approximately 2-5 minutes.

Features:
- **Default**: CPU proving (slower but portable)
- **cuda**: NVIDIA GPU acceleration (use for production)
- **metal**: Apple GPU acceleration (for local development on Mac)

## Deployment

### Local Development

```bash
# Build (CPU proving)
cargo build --release -p proof-service

# Build with CUDA support
cargo build --release -p proof-service --features cuda

# Run
./target/release/proof-service
```

### Docker

```bash
# Build image with CUDA support
docker build -f Dockerfile.risc0 -t proof-service .

# Run with GPU
docker run --gpus all -p 8083:8083 proof-service
```

### Cloud Run

The service is deployed via Terraform in `deploy/terraform/modules/proof-service/`. See the staging environment for configuration examples.

Key settings:
- L4 GPU for proof generation
- 60-minute timeout for proof generation
- Single instance, concurrency of 1

## Security Considerations

1. **Rate Limiting**: Each proof takes 2-5 minutes. Consider rate limiting or authentication.

2. **Network Security**: The service should only be accessible from TEE workloads. Consider VPC Service Controls or mTLS.

3. **Attestation Freshness**: JWT tokens expire after 1 hour. The proof embeds the validity window.

4. **Image Signature Verification**: The image signature is embedded in the proof and verified on-chain using ecrecover. The on-chain contract maintains a registry of trusted signer addresses.

## ImageId and Build Determinism

The RISC Zero `imageId` is a hash of the compiled guest ELF. **The imageId used on-chain must match the imageId embedded in the deployed binary exactly.**

### Why Build-Time Extraction?

The guest ELF is compiled inside the Docker container during `cargo build`. If you build separately on the CI runner, different build environments produce different ELF binaries → different imageIds → proof verification fails on-chain.

### CUDA Stubs

The binary is linked against CUDA and requires `libcuda.so.1` to start. This library is the CUDA driver provided by the host via `--gpus`, not bundled in images. CI has no GPU.

**Solution**: Extract the imageId during Docker build using CUDA stubs:

```dockerfile
# In builder stage (nvidia/cuda:12.2.0-devel has stubs at /usr/local/cuda/lib64/stubs/)
# build.rs writes imageId to risc0_image_id.txt during cargo build
```

CI then reads `/app/risc0_image_id.txt` from the built image instead of running the binary.

### Debugging Verification Failures

If proofs verify locally but fail on-chain with `VerificationFailed()`:

1. Check the on-chain contract's imageId matches the deployed proof-service
2. Verify the imageId in GCP Artifact Registry OCI artifact matches
3. Ensure no separate build generated a different imageId

## Development

### Prerequisites

- Rust 1.75+
- RISC Zero toolchain: `curl -L https://risczero.com/install | bash && rzup install`

### Building the RISC Zero Program

The RISC Zero program is built automatically via `build.rs`:

```bash
# The program is at crates/synddb-bootstrap/risc0/program/
cargo build -p proof-service
```

### Testing

The proof-service can be tested locally using CPU proving (no GPU required):

```bash
cargo run -p proof-service
```

For GPU testing, ensure CUDA is installed and use the `cuda` feature.
