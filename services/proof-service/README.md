# Proof Service

SP1 proof generation service for TEE attestation verification using the Succinct Network Prover.

This service receives GCP Confidential Space attestation tokens from TEE services (sequencers/validators) and generates SP1 zero-knowledge proofs that can be verified on-chain. The proofs attest that:

1. The JWT token was signed by Google's JWKS keys
2. The TEE is running the expected container image
3. Secure boot is enabled and debug mode is disabled
4. The cosign signature over the image digest is valid

## Architecture

```
TEE Service                    Proof Service                  On-Chain
    │                              │                              │
    │  POST /prove                 │                              │
    │  {jwt_token, audience,       │                              │
    │   evm_public_key,            │                              │
    │   cosign_signature,          │                              │
    │   cosign_pubkey}             │                              │
    ├─────────────────────────────►│                              │
    │                              │  Fetch JWKS from Google      │
    │                              │  Generate SP1 proof (network)│
    │                              │                              │
    │  {public_values, proof,      │                              │
    │   tee_address}               │                              │
    │◄─────────────────────────────┤                              │
    │                              │                              │
    │  Register key via relayer    │                              │
    │  (public_values, proof)      │                              │
    ├─────────────────────────────────────────────────────────────►│
    │                              │                  Verify proof │
    │                              │         Verify cosign (P256) │
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
  "cosign_signature": "0x...",
  "cosign_pubkey": "0x..."
}
```

| Field | Type | Description |
|-------|------|-------------|
| `jwt_token` | string | Raw JWT attestation token from GCP Confidential Space |
| `expected_audience` | string | Expected audience claim in the JWT |
| `evm_public_key` | hex string | 64-byte uncompressed secp256k1 public key (no 0x04 prefix) |
| `cosign_signature` | hex string | 64-byte ECDSA P-256 signature (r \|\| s) over image digest |
| `cosign_pubkey` | hex string | 64 or 65-byte P-256 public key (x \|\| y or 0x04 \|\| x \|\| y) |

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
| `public_values` | hex string | ABI-encoded `PublicValuesStruct` for on-chain verification |
| `proof_bytes` | hex string | SP1 Groth16 proof bytes |
| `tee_address` | address | Ethereum address derived from the EVM public key |

**Response (503):** Prover is busy generating another proof.

**Response (500):** Proof generation failed (check `error` and `details` fields).

### GET /health

Health check endpoint.

**Response:**
```json
{
  "status": "ready",
  "prover_busy": false
}
```

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
    bytes32 cosign_signature_r;   // Cosign signature R component (P-256)
    bytes32 cosign_signature_s;   // Cosign signature S component (P-256)
    bytes32 cosign_pubkey_x;      // Cosign public key X coordinate (P-256)
    bytes32 cosign_pubkey_y;      // Cosign public key Y coordinate (P-256)
}
```

The on-chain `AttestationVerifier` contract:
1. Verifies the SP1 proof using the verification key
2. Checks the JWK hash is trusted (Google's signing keys)
3. Validates the timestamp window
4. Requires secure boot and production mode
5. Verifies the image digest matches the expected value
6. Verifies the cosign signature using the RIP-7212 P256 precompile
7. Checks the cosign public key is in the trusted registry

## Configuration

| Environment Variable | Default | Description |
|---------------------|---------|-------------|
| `BIND_ADDRESS` | `0.0.0.0:8080` | HTTP server listen address |
| `LOG_JSON` | `false` | Enable JSON log output |
| `SP1_PROVER` | `network` | Prover backend (network only) |
| `NETWORK_PRIVATE_KEY` | (required) | Secp256k1 private key for SP1 Network Prover |
| `GOOGLE_OIDC_DISCOVERY_URL` | GCP default | OIDC discovery endpoint for JWKS |
| `JWKS_CACHE_TTL_SECS` | `3600` | How long to cache Google's public keys |

### SP1 Network Prover

This service uses the [SP1 Network Prover](https://docs.succinct.xyz/prover-network/overview.html), which offloads proof generation to Succinct's hosted infrastructure. This requires:

1. A secp256k1 private key with PROVE tokens (set via `NETWORK_PRIVATE_KEY`)
2. Network connectivity to Succinct's prover network

Proof generation typically takes 2-5 minutes depending on network load.

## Deployment

### Local Development

```bash
# Build
cargo build --release

# Run with network prover
NETWORK_PRIVATE_KEY=<your-key> ./target/release/proof-service
```

### Docker

```bash
# Build image
docker build -t proof-service .

# Run
docker run -p 8080:8080 -e NETWORK_PRIVATE_KEY=<key> proof-service
```

### Cloud Run

The service is deployed via Terraform in `deploy/terraform/modules/proof-service/`. See the staging environment for configuration examples.

Key settings:
- 1 vCPU, 512MB RAM (lightweight since proving is offloaded)
- 60-minute timeout for proof generation
- Single instance, concurrency of 1

## Security Considerations

1. **Rate Limiting**: Each proof takes 2-5 minutes. Consider rate limiting or authentication.

2. **Network Security**: The service should only be accessible from TEE workloads. Consider VPC Service Controls or mTLS.

3. **Attestation Freshness**: JWT tokens expire after 1 hour. The proof embeds the validity window.

4. **Cosign Verification**: The cosign signature is passed through the proof and verified on-chain using the P256 precompile. The on-chain contract maintains a registry of trusted cosign public keys.

## Development

### Prerequisites

- Rust 1.75+
- SP1 toolchain: `curl -L https://sp1.succinct.xyz | bash && sp1up`

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
