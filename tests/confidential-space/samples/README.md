# Attestation Samples

This directory contains attestation token samples captured from a GCP Confidential Space VM.

## How These Samples Were Generated

1. **Deploy the workload** to GCP Confidential Space:
   ```bash
   cd tests/confidential-space
   ./deploy.sh <project-id> us-central1 --run
   ```
   This builds the Docker image, pushes it to Artifact Registry, and creates a Confidential VM that runs the attestation capture workload.

2. **Wait for completion** (~1-2 minutes). The workload automatically:
   - Requests attestation tokens from the TEE server
   - Fetches Google's JWKS public keys
   - Uploads the bundle to GCS

3. **Download the samples** from GCS (as shown in deploy.sh output):
   ```bash
   mkdir -p samples
   gcloud storage cp 'gs://<project-id>-cs-attestation-samples/attestation-samples/*' ./samples/
   ```

The `deploy.sh` script prints these exact commands after the VM is created.

## Sample Contents

Each JSON file contains:

| Field | Description |
|-------|-------------|
| `samples[]` | Array of captured attestation tokens |
| `samples[].raw_token` | The complete JWT (header.payload.signature) |
| `samples[].header` | Decoded JWT header (alg, kid, typ) |
| `samples[].claims` | Decoded JWT payload with TEE attestation claims |
| `samples[].signature_bytes` | RS256 signature (hex-encoded) |
| `samples[].signing_input` | The signed data: base64url(header).base64url(payload) |
| `jwks` | Google's public keys for signature verification |
| `oidc_discovery` | OIDC configuration from Google |
| `instructions` | Verification steps for developers |

## Key Claims

The attestation tokens contain claims that prove the workload is running in a genuine Confidential Space VM:

- `iss`: `https://confidentialcomputing.googleapis.com` (Google's attestation service)
- `hwmodel`: `GCP_AMD_SEV` (AMD SEV hardware)
- `swname`: `CONFIDENTIAL_SPACE` (Confidential Space software)
- `secboot`: `true` (Secure Boot enabled)
- `submods.container.image_digest`: SHA256 hash of the container image

## Usage

These samples are used for developing and testing RISC Zero on-chain attestation verification without needing to run a Confidential Space VM every time.

## Verifying Samples

### Option 1: Using the gcp-attestation library

```bash
cargo test -p gcp-attestation
```

### Option 2: Proof-service tests (includes guest program execution)

```bash
cargo test -p proof-service --release
```

### Option 3: RISC Zero proof generation

Use the proof-service to generate proofs from these attestation samples. See `crates/proof-service/README.md` for details.
