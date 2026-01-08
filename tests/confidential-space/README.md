# Confidential Space Attestation Sample Capture

This directory contains tooling for GCP Confidential Space TEE attestation:

1. **Sample capture workload** - Runs in GCP CS VM to capture real attestation tokens
2. **Verification library** - `crates/gcp-attestation/` crate for verifying attestations
3. **RISC Zero integration** - `crates/synddb-bootstrap/risc0/` for generating ZK proofs of attestation

## Directory Structure

```
tests/confidential-space/
├── src/
│   └── main.rs          # Attestation capture workload (runs in GCP CS VM)
├── samples/             # Captured attestation tokens
├── deploy.sh            # Build and deploy to GCP
├── setup-gcp.sh         # One-time GCP infrastructure setup
└── Dockerfile

# Related crates:
crates/gcp-attestation/                    # Core attestation verification library
crates/synddb-bootstrap/risc0/program/     # RISC Zero zkVM program
crates/proof-service/                      # GPU proof generation service
```

## Quick Start

### 1. Capture Attestation Samples

```bash
# One-time setup (creates GCP resources)
./setup-gcp.sh <your-project-id> us-central1

# Build, push, and run the workload
./deploy.sh <your-project-id> us-central1 --run

# Wait ~2 minutes, then download the attestation samples
mkdir -p samples
gcloud storage cp 'gs://<your-project-id>-cs-attestation-samples/attestation-samples/*' ./samples/
```

### 2. Verify Samples Locally

```bash
# Run gcp-attestation library tests
cargo test -p gcp-attestation

# Or run proof-service tests (includes guest program execution)
cargo test -p proof-service --release
```

### 3. Generate RISC Zero ZK Proof

The proof generation is done by the proof-service. See `crates/proof-service/README.md` for details.

---

## What It Does

1. Runs inside a Confidential Space VM (AMD SEV-SNP)
2. Fetches attestation tokens from the local TEE attestation service
3. Captures the raw JWT tokens with decoded headers/claims
4. Fetches Google's JWKS (public keys) for signature verification
5. Outputs everything as JSON for RISC Zero development

## Output Format

The workload produces a JSON bundle like:

```json
{
  "samples": [
    {
      "raw_token": "eyJhbGciOiJSUzI1NiIsImtpZCI6IjEyMzQ1In0.eyJpc3MiOi...",
      "header": { "alg": "RS256", "kid": "12345" },
      "claims": {
        "iss": "https://confidentialcomputing.googleapis.com",
        "aud": "https://synddb-sequencer.example.com",
        "exp": 1733180000,
        "secboot": true,
        "swname": "CONFIDENTIAL_SPACE",
        "submods": {
          "container": { "image_digest": "sha256:..." }
        }
      },
      "signature_bytes": "0x...",
      "signing_input": "eyJhbGci...eyJpc3Mi..."
    }
  ],
  "jwks": {
    "keys": [{ "kty": "RSA", "kid": "12345", "n": "...", "e": "AQAB" }]
  }
}
```

## RISC Zero On-Chain Verification

The RISC Zero program verifies attestations inside a zkVM and produces public values for on-chain verification:

| Public Value | Description |
|--------------|-------------|
| `jwk_key_hash` | keccak256 of Google's signing key ID |
| `validity_window_start` | Token issued-at timestamp |
| `validity_window_end` | Token expiration timestamp |
| `image_digest_hash` | keccak256 of container image digest |
| `tee_signing_key` | Ethereum address derived from EVM public key |
| `secboot` | Whether secure boot was enabled |
| `audience_hash` | keccak256 of audience string |
| `image_signature_*` | secp256k1 signature components (r, s, v) |

### Hand-off to Solidity Developer

After generating proofs, provide:

1. **Public values**: ABI-encoded `PublicValuesStruct` from the proof output
2. **Proof bytes**: RISC Zero Groth16 seal
3. **Image ID**: Get via `GET /image-id` on the proof service
4. **PublicValuesStruct**: Defined in `crates/synddb-bootstrap/risc0/program/src/types.rs`

---

## Manual Deployment Steps

### Setup GCP Infrastructure

```bash
PROJECT_ID=<your-project-id>
REGION=us-central1

# Enable APIs
gcloud services enable \
    artifactregistry.googleapis.com \
    compute.googleapis.com \
    confidentialcomputing.googleapis.com

# Create service account
gcloud iam service-accounts create cs-attestation-workload

# Grant required roles
gcloud projects add-iam-policy-binding $PROJECT_ID \
    --member="serviceAccount:cs-attestation-workload@$PROJECT_ID.iam.gserviceaccount.com" \
    --role="roles/confidentialcomputing.workloadUser"
```

### Build & Push

```bash
# Configure docker
gcloud auth configure-docker ${REGION}-docker.pkg.dev

# Build
docker build -t ${REGION}-docker.pkg.dev/${PROJECT_ID}/synddb-test/cs-attestation-sample:latest .

# Push
docker push ${REGION}-docker.pkg.dev/${PROJECT_ID}/synddb-test/cs-attestation-sample:latest
```

### Run

```bash
# Create Confidential VM
gcloud compute instances create cs-attestation-vm \
    --zone=${REGION}-a \
    --machine-type=n2d-standard-2 \
    --confidential-compute-type=SEV \
    --shielded-secure-boot \
    --scopes=cloud-platform \
    --image-project=confidential-space-images \
    --image-family=confidential-space-debug \
    --service-account=cs-attestation-workload@${PROJECT_ID}.iam.gserviceaccount.com \
    --metadata="tee-image-reference=${REGION}-docker.pkg.dev/${PROJECT_ID}/synddb-test/cs-attestation-sample:latest,tee-container-log-redirect=true"
```

## Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `ATTESTATION_AUDIENCE` | Audience claim for tokens | `https://synddb-sequencer.example.com` |
| `OUTPUT_BUCKET` | GCS bucket for uploading samples | (none) |
| `RUST_LOG` | Log level filter | `info` |

## Cost Notes

- Confidential VMs require N2D instances with AMD SEV (minimum: n2d-standard-2)
- **Spot instances** are used by default for ~60-90% cost savings
- The workload completes quickly (~1-2 minutes)

## Troubleshooting

### Workload not starting

Check the launcher logs:
```bash
gcloud logging read 'logName="projects/<project>/logs/confidential-space-launcher"' --limit=20
```

### Attestation socket not found

The workload must run inside Confidential Space. It will fail immediately if run locally.

### Permission denied on GCS

Ensure the service account has `roles/storage.objectAdmin` on the bucket.
