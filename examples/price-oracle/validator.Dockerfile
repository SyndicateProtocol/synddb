# Price Oracle Custom Validator
#
# This validator extends synddb-validator with price consistency rules.
# It validates that prices from multiple sources agree within tolerance.

FROM rust:1.92-trixie AS builder

# Install clang for bindgen (needed for rusqlite bundled feature)
RUN apt-get update && apt-get install -y clang && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy workspace files
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY examples ./examples
COPY tests ./tests

# Build the custom validator
RUN cargo build --release -p price-oracle-validator

# Runtime stage - minimal image
FROM debian:trixie-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Copy the validator binary
COPY --from=builder /app/target/release/price-oracle-validator /usr/local/bin/

# Default environment
ENV RUST_LOG=info
ENV BIND_ADDRESS=0.0.0.0:8080

EXPOSE 8080

# Confidential Space labels
# allow_env_override: Environment variables that can be set by the VM operator
# Core: RUST_LOG, BIND_ADDRESS, DATABASE_PATH, STATE_DB_PATH, SEQUENCER_URL
# Storage: GCS_BUCKET, GCS_PREFIX, FETCHER_TYPE, DA_PROVIDER
# Sync: SYNC_INTERVAL, BATCH_SYNC_ENABLED
# Bridge: BRIDGE_SIGNER, BRIDGE_CONTRACT_ADDRESS, BRIDGE_CHAIN_ID
# TEE Bootstrap: ENABLE_KEY_BOOTSTRAP, TEE_KEY_MANAGER_CONTRACT_ADDRESS, BOOTSTRAP_RPC_URL, BOOTSTRAP_CHAIN_ID, PROOF_SERVICE_URL, ATTESTATION_AUDIENCE
# Logging: LOG_JSON
LABEL "tee.launch_policy.allow_env_override"="RUST_LOG,BIND_ADDRESS,DATABASE_PATH,STATE_DB_PATH,SEQUENCER_URL,GCS_BUCKET,GCS_PREFIX,FETCHER_TYPE,DA_PROVIDER,SYNC_INTERVAL,BATCH_SYNC_ENABLED,BRIDGE_SIGNER,BRIDGE_CONTRACT_ADDRESS,BRIDGE_CHAIN_ID,ENABLE_KEY_BOOTSTRAP,TEE_KEY_MANAGER_CONTRACT_ADDRESS,BOOTSTRAP_RPC_URL,BOOTSTRAP_CHAIN_ID,PROOF_SERVICE_URL,ATTESTATION_AUDIENCE,LOG_JSON"
LABEL "tee.launch_policy.log_redirect"="always"

ENTRYPOINT ["price-oracle-validator"]
