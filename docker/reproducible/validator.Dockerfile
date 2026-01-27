# Reproducible Build - SyndDB Validator
#
# This Dockerfile produces bit-for-bit reproducible builds for verification
# in Google Cloud Confidential Space.
#
# Targets:
#   release (default) - Distroless image for production/TEE deployment
#   debug             - Debian-slim image with shell for debugging
#
# Build for production:
#   docker build -f docker/reproducible/validator.Dockerfile \
#     --platform linux/amd64 \
#     -t synddb-validator:reproducible .
#
# Build for debugging:
#   docker build -f docker/reproducible/validator.Dockerfile \
#     --platform linux/amd64 \
#     --target debug \
#     -t synddb-validator:debug .
#
# Verify image hash:
#   docker inspect synddb-validator:reproducible --format='{{.Id}}'
#
# Key reproducibility features:
#   - Base images pinned by digest (not tag)
#   - SOURCE_DATE_EPOCH=0 for deterministic timestamps
#   - --remap-path-prefix normalizes build paths
#   - cargo --locked ensures exact dependency versions
#   - Stripped binaries remove variable metadata

# =============================================================================
# Build Arguments - Update these when upgrading base images
# =============================================================================
# To get current digests:
#   docker pull rust:1.92-trixie && docker inspect rust:1.92-trixie --format='{{index .RepoDigests 0}}'
#   docker pull debian:trixie-slim && docker inspect debian:trixie-slim --format='{{index .RepoDigests 0}}'
#   docker pull gcr.io/distroless/cc-debian13 && docker inspect gcr.io/distroless/cc-debian13 --format='{{index .RepoDigests 0}}'

ARG RUST_IMAGE_DIGEST=sha256:48851a839d6a67370c9dbe0e709bedc138e3e404b161c5233aedcf2b717366e4
ARG DEBIAN_IMAGE_DIGEST=sha256:e711a7b30ec1261130d0a121050b4ed81d7fb28aeabcf4ea0c7876d4e9f5aca2
ARG DISTROLESS_IMAGE_DIGEST=sha256:43fc7a7004c4cdb27aac60b3e95c87130cf47823f72d25d42ed0f9b503f1d184
ARG DEBIAN_SNAPSHOT=20260104T000000Z

# =============================================================================
# Builder Stage
# =============================================================================
FROM --platform=linux/amd64 rust@${RUST_IMAGE_DIGEST} AS builder

# Re-declare ARG after FROM for use in this stage
ARG DEBIAN_SNAPSHOT

# Reproducibility: Set epoch time for all timestamps
ENV SOURCE_DATE_EPOCH=0

# Reproducibility: Disable incremental compilation (non-deterministic)
ENV CARGO_INCREMENTAL=0

# Reproducibility: Force single-threaded compilation for deterministic order
ENV CARGO_BUILD_JOBS=1

# Reproducibility: Fixed cargo home for consistent paths
ENV CARGO_HOME=/cargo

# Reproducibility: Remap source paths for consistent debug info
ENV RUSTFLAGS="--remap-path-prefix=/app=/build --remap-path-prefix=/cargo=/cargo"

# Reproducibility: Use fixed hash seed
ENV RUSTC_HASH_UNTRACKED_METADATA=1

# Reproducibility: Pin apt packages to a specific snapshot date
# This ensures the same package versions are always installed, even years later
RUN echo "deb [check-valid-until=no] https://snapshot.debian.org/archive/debian/${DEBIAN_SNAPSHOT} trixie main" > /etc/apt/sources.list && \
    echo "deb [check-valid-until=no] https://snapshot.debian.org/archive/debian-security/${DEBIAN_SNAPSHOT} trixie-security main" >> /etc/apt/sources.list && \
    apt-get update && apt-get install -y --no-install-recommends \
    clang \
    libclang-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy dependency manifests first for layer caching
COPY Cargo.toml Cargo.lock ./

# Copy workspace crates
COPY crates/ ./crates/
COPY examples/ ./examples/
COPY tests/ ./tests/

# Normalize timestamps on all source files for reproducibility
RUN find /app -type f -exec touch -d "@0" {} +

# Build with reproducible profile (single codegen unit, LTO, panic=abort)
# --locked ensures Cargo.lock is respected exactly
RUN cargo build --profile reproducible --locked -p synddb-validator --features tee

# Strip the binary for consistent output (removes variable metadata)
RUN strip --strip-all /app/target/reproducible/synddb-validator

# Normalize the binary timestamp
RUN touch -d "@0" /app/target/reproducible/synddb-validator

# =============================================================================
# Debug Runtime - Debian with shell for troubleshooting
# =============================================================================
FROM --platform=linux/amd64 debian@${DEBIAN_IMAGE_DIGEST} AS debug

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    procps \
    sqlite3 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /app/target/reproducible/synddb-validator /app/synddb-validator

RUN mkdir -p /data

ENV RUST_LOG=info
ENV BIND_ADDRESS=0.0.0.0:8080
ENV DATABASE_PATH=/data/validator.db
ENV STATE_DB_PATH=/data/validator_state.db

EXPOSE 8080

ENTRYPOINT ["/app/synddb-validator"]

# =============================================================================
# Release Runtime - Distroless for production/TEE
# =============================================================================
FROM --platform=linux/amd64 gcr.io/distroless/cc-debian13@${DISTROLESS_IMAGE_DIGEST} AS release

WORKDIR /app

COPY --from=builder /app/target/reproducible/synddb-validator /app/synddb-validator

ENV RUST_LOG=info
ENV BIND_ADDRESS=0.0.0.0:8080
ENV DATABASE_PATH=/data/validator.db
ENV STATE_DB_PATH=/data/validator_state.db

EXPOSE 8080

# Confidential Space labels
# allow_env_override: Environment variables that can be set by the VM operator
# Core: RUST_LOG, BIND_ADDRESS, DATABASE_PATH, STATE_DB_PATH, SEQUENCER_URL
# Storage: GCS_BUCKET, GCS_PREFIX, FETCHER_TYPE, DA_PROVIDER
# Sync: SYNC_INTERVAL, BATCH_SYNC_ENABLED
# Bridge: BRIDGE_SIGNER, BRIDGE_CONTRACT_ADDRESS, BRIDGE_CHAIN_ID
# TEE Bootstrap: ENABLE_KEY_BOOTSTRAP, TEE_KEY_MANAGER_CONTRACT_ADDRESS, BOOTSTRAP_RPC_URL, BOOTSTRAP_CHAIN_ID, PROOF_SERVICE_URL, ATTESTATION_AUDIENCE, RELAYER_URL
# Logging: LOG_JSON
LABEL "tee.launch_policy.allow_env_override"="RUST_LOG,BIND_ADDRESS,DATABASE_PATH,STATE_DB_PATH,SEQUENCER_URL,GCS_BUCKET,GCS_PREFIX,FETCHER_TYPE,DA_PROVIDER,SYNC_INTERVAL,BATCH_SYNC_ENABLED,BRIDGE_SIGNER,BRIDGE_CONTRACT_ADDRESS,BRIDGE_CHAIN_ID,ENABLE_KEY_BOOTSTRAP,TEE_KEY_MANAGER_CONTRACT_ADDRESS,BOOTSTRAP_RPC_URL,BOOTSTRAP_CHAIN_ID,PROOF_SERVICE_URL,ATTESTATION_AUDIENCE,RELAYER_URL,LOG_JSON"
LABEL "tee.launch_policy.log_redirect"="always"

ENTRYPOINT ["/app/synddb-validator"]
