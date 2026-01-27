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

ENTRYPOINT ["price-oracle-validator"]
