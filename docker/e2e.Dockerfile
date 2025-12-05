# E2E Test Runner Dockerfile
#
# Builds the synddb-e2e test runner for running end-to-end tests
# against sequencer and validator services.

# Build stage
FROM rust:1.91-bookworm AS builder

WORKDIR /app

# Copy workspace manifests
COPY Cargo.toml Cargo.lock ./

# Copy all crates and tests
COPY crates/ ./crates/
COPY tests/ ./tests/

# Build the e2e binary
RUN cargo build --release -p synddb-e2e

# Runtime stage
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy the binary
COPY --from=builder /app/target/release/synddb-e2e /app/synddb-e2e

ENV RUST_LOG=info

ENTRYPOINT ["/app/synddb-e2e"]
