# Build stage - explicitly target linux/amd64 for Confidential Space VMs
FROM --platform=linux/amd64 rust:1.91-bookworm AS builder

WORKDIR /app

# Copy manifests
COPY Cargo.toml Cargo.lock* ./
COPY src ./src

# Build release binary with GCS support
RUN cargo build --release --features gcs

# Runtime stage - use distroless for minimal attack surface
# Note: Using root variant because the attestation socket requires root access
FROM --platform=linux/amd64 gcr.io/distroless/cc-debian12

WORKDIR /app

# Copy the binary
COPY --from=builder /app/target/release/cs-attestation-sample /app/cs-attestation-sample

# The attestation socket is at /run/container_launcher/teeserver.sock
# This is automatically available in Confidential Space

# Default environment variables
ENV RUST_LOG=info
ENV ATTESTATION_AUDIENCE=https://synddb-sequencer.example.com

# Confidential Space labels - declare which env vars can be overridden by the operator
# See: https://cloud.google.com/confidential-computing/confidential-space/docs/create-customize-workloads
LABEL "tee.launch_policy.allow_env_override"="ATTESTATION_AUDIENCE,OUTPUT_BUCKET,RUST_LOG,RUST_LOG_FORMAT"
LABEL "tee.launch_policy.log_redirect"="always"

# Run the binary
ENTRYPOINT ["/app/cs-attestation-sample"]
