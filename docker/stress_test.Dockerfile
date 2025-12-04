# Session Race Condition Stress Test Dockerfile
#
# This builds the session-stress-test binary which attempts to trigger
# the SIGSEGV caused by unsafe impl Send for SessionState.
#
# Usage:
#   docker build -f docker/stress_test.Dockerfile -t synddb-stress-test .
#   docker run --rm --network synddb_default -e SEQUENCER_URL=http://sequencer:8433 synddb-stress-test

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

# Build the stress test binary
RUN cargo build --release -p synddb-benchmark --bin session-stress-test

# Runtime stage
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy the binary
COPY --from=builder /app/target/release/session-stress-test /app/session-stress-test

ENV RUST_LOG=info

ENTRYPOINT ["/app/session-stress-test"]
