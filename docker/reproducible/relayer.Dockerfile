# SyndDB Relayer
#
# Build: docker build -f docker/reproducible/relayer.Dockerfile -t synddb-relayer .
# Run:   docker run -p 8082:8082 synddb-relayer

# Build stage
FROM rust:1.92-bookworm AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    clang \
    libclang-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY crates/ ./crates/
COPY examples/ ./examples/
COPY tests/ ./tests/

RUN cargo build --release --locked -p synddb-relayer

# Runtime stage
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /app/target/release/synddb-relayer /app/synddb-relayer

ENV RUST_LOG=info
ENV RELAYER_LISTEN_ADDR=0.0.0.0:8082

EXPOSE 8082

ENTRYPOINT ["/app/synddb-relayer"]
