# Customer SQLite Application Dockerfile
#
# This Dockerfile builds the orderbook benchmark with synddb-client integration.
# Replace this file with your own application's Dockerfile.
#
# The benchmark automatically uses synddb-client when SEQUENCER_URL is set.
# Changesets are captured from SQLite and sent to the sequencer for ordering.
#
# Requirements for your own app:
# - Use synddb-client to wrap your SQLite connection
# - Set SEQUENCER_URL environment variable (e.g., http://sequencer:8433)

# Build stage
FROM rust:1.91-bookworm AS builder

# Install clang for libsqlite3-sys bindgen
RUN apt-get update && apt-get install -y --no-install-recommends \
    clang \
    libclang-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy workspace manifests
COPY Cargo.toml Cargo.lock ./

# Copy all crates
COPY crates/ ./crates/

# Build the benchmark binary
RUN cargo build --release -p synddb-benchmark

# Runtime stage
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy the binary (orderbook-bench is the binary name from synddb-benchmark)
COPY --from=builder /app/target/release/orderbook-bench /app/orderbook-bench

# Create data directory
RUN mkdir -p /data

ENV RUST_LOG=info

ENTRYPOINT ["/app/orderbook-bench"]
