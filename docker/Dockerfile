# Multi-Service Dockerfile for SyndDB
#
# This Dockerfile uses multi-stage builds to compile all binaries once
# and produce minimal runtime images for each service.
#
# Usage with docker compose:
#   build:
#     context: .
#     dockerfile: docker/Dockerfile
#     target: sequencer
#
# Available targets:
#   - sequencer    : SyndDB sequencer service (includes all transports)
#   - validator    : SyndDB validator service
#   - customer-app : Orderbook benchmark (example customer application)
#   - e2e          : E2E test runner
#   - stress-test  : Session stress test

# =============================================================================
# Builder Stage - Compiles all workspace binaries
# =============================================================================
FROM rust:1.91-bookworm AS builder

# Install clang for libsqlite3-sys bindgen
RUN apt-get update && apt-get install -y --no-install-recommends \
    clang \
    libclang-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy workspace manifests
COPY Cargo.toml Cargo.lock ./

# Copy all crates, examples, and tests
COPY crates/ ./crates/
COPY examples/ ./examples/
COPY tests/ ./tests/

# Build all workspace binaries in release mode
RUN cargo build --release --workspace

# =============================================================================
# Runtime Base - Common runtime dependencies
# =============================================================================
FROM debian:bookworm-slim AS runtime-base

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# =============================================================================
# Sequencer Service
# =============================================================================
FROM runtime-base AS sequencer

RUN apt-get update && apt-get install -y --no-install-recommends \
    wget \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/synddb-sequencer /app/synddb-sequencer

RUN mkdir -p /data

ENV RUST_LOG=info
ENV BIND_ADDRESS=0.0.0.0:8433

EXPOSE 8433

ENTRYPOINT ["/app/synddb-sequencer"]

# =============================================================================
# Validator Service
# =============================================================================
FROM runtime-base AS validator

RUN apt-get update && apt-get install -y --no-install-recommends \
    wget \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/synddb-validator /app/synddb-validator

RUN mkdir -p /data

ENV RUST_LOG=info
ENV BIND_ADDRESS=0.0.0.0:8080
ENV DATABASE_PATH=/data/validator.db
ENV STATE_DB_PATH=/data/validator_state.db

EXPOSE 8080

ENTRYPOINT ["/app/synddb-validator"]

# =============================================================================
# Customer App (Orderbook Benchmark)
# =============================================================================
FROM runtime-base AS customer-app

COPY --from=builder /app/target/release/orderbook-bench /app/orderbook-bench

RUN mkdir -p /data

ENV RUST_LOG=info

ENTRYPOINT ["/app/orderbook-bench"]

# =============================================================================
# E2E Test Runner
# =============================================================================
FROM runtime-base AS e2e

COPY --from=builder /app/target/release/synddb-e2e /app/synddb-e2e

ENV RUST_LOG=info

ENTRYPOINT ["/app/synddb-e2e"]

# =============================================================================
# Session Stress Test
# =============================================================================
FROM runtime-base AS stress-test

COPY --from=builder /app/target/release/session-stress-test /app/session-stress-test

ENV RUST_LOG=info

ENTRYPOINT ["/app/session-stress-test"]
