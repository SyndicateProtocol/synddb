# Reproducible Build - SyndDB Sequencer
#
# This Dockerfile produces bit-for-bit reproducible builds for verification
# in Google Cloud Confidential Space.
#
# Targets:
#   release (default) - Distroless image for production/TEE deployment
#   debug             - Debian-slim image with shell for debugging
#
# Build for production:
#   docker build -f docker/reproducible/sequencer.Dockerfile \
#     --platform linux/amd64 \
#     -t synddb-sequencer:reproducible .
#
# Build for debugging:
#   docker build -f docker/reproducible/sequencer.Dockerfile \
#     --platform linux/amd64 \
#     --target debug \
#     -t synddb-sequencer:debug .
#
# Verify image hash:
#   docker inspect synddb-sequencer:reproducible --format='{{.Id}}'
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
#   docker pull rust:1.92-bookworm && docker inspect rust:1.92-bookworm --format='{{index .RepoDigests 0}}'
#   docker pull debian:bookworm-slim && docker inspect debian:bookworm-slim --format='{{index .RepoDigests 0}}'
#   docker pull gcr.io/distroless/cc-debian12 && docker inspect gcr.io/distroless/cc-debian12 --format='{{index .RepoDigests 0}}'

ARG RUST_IMAGE_DIGEST=sha256:9676d0547a259997add8f5924eb6b959c589ed39055338e23b99aba7958d6d31
ARG DEBIAN_IMAGE_DIGEST=sha256:e899040a73d36e2b36fa33216943539d9957cba8172b858097c2cabcdb20a3e2
ARG DISTROLESS_IMAGE_DIGEST=sha256:0c8eac8ea42a167255d03c3ba6dfad2989c15427ed93d16c53ef9706ea4691df

# =============================================================================
# Builder Stage
# =============================================================================
FROM --platform=linux/amd64 rust@${RUST_IMAGE_DIGEST} AS builder

# Reproducibility: Set epoch time for all timestamps
ENV SOURCE_DATE_EPOCH=0

# Reproducibility: Disable incremental compilation (non-deterministic)
ENV CARGO_INCREMENTAL=0

# Reproducibility: Remap source paths to /build for consistent debug info
ENV RUSTFLAGS="--remap-path-prefix=/app=/build --remap-path-prefix=/usr/local/cargo=/cargo"

# Install build dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
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

# Build with locked dependencies and release optimizations
RUN cargo build --release --locked -p synddb-sequencer

# Strip the binary for consistent output
RUN strip --strip-all /app/target/release/synddb-sequencer

# =============================================================================
# Debug Runtime - Debian with shell for troubleshooting
# =============================================================================
FROM --platform=linux/amd64 debian@${DEBIAN_IMAGE_DIGEST} AS debug

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    procps \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /app/target/release/synddb-sequencer /app/synddb-sequencer

RUN mkdir -p /data

ENV RUST_LOG=info
ENV BIND_ADDRESS=0.0.0.0:8433

EXPOSE 8433

ENTRYPOINT ["/app/synddb-sequencer"]

# =============================================================================
# Release Runtime - Distroless for production/TEE
# =============================================================================
FROM --platform=linux/amd64 gcr.io/distroless/cc-debian12@${DISTROLESS_IMAGE_DIGEST} AS release

WORKDIR /app

COPY --from=builder /app/target/release/synddb-sequencer /app/synddb-sequencer

ENV RUST_LOG=info
ENV BIND_ADDRESS=0.0.0.0:8433

EXPOSE 8433

# Confidential Space labels
LABEL "tee.launch_policy.allow_env_override"="RUST_LOG,BIND_ADDRESS,SIGNING_KEY,GCS_BUCKET"
LABEL "tee.launch_policy.log_redirect"="always"

ENTRYPOINT ["/app/synddb-sequencer"]
