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

# Build the sequencer binary
RUN cargo build --release -p synddb-sequencer

# Runtime stage
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    wget \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy the binary
COPY --from=builder /app/target/release/synddb-sequencer /app/synddb-sequencer

# Create data directory
RUN mkdir -p /data

ENV RUST_LOG=info
ENV BIND_ADDRESS=0.0.0.0:8433

EXPOSE 8433

ENTRYPOINT ["/app/synddb-sequencer"]
