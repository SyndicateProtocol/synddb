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

# Copy all crates and tests
COPY crates/ ./crates/
COPY tests/ ./tests/

# Build the validator binary
RUN cargo build --release -p synddb-validator

# Runtime stage
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    wget \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy the binary
COPY --from=builder /app/target/release/synddb-validator /app/synddb-validator

# Create data directory
RUN mkdir -p /data

ENV RUST_LOG=info
ENV BIND_ADDRESS=0.0.0.0:8080
ENV DATABASE_PATH=/data/validator.db
ENV STATE_DB_PATH=/data/validator_state.db

EXPOSE 8080

ENTRYPOINT ["/app/synddb-validator"]
