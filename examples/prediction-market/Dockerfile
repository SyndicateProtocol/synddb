# Build stage
FROM rust:1.91-bookworm AS builder

# Install clang for bindgen (needed for rusqlite bundled feature)
RUN apt-get update && apt-get install -y clang && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy all workspace members needed for cargo to resolve
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY examples ./examples
COPY tests ./tests

# Build the prediction-market binary and sequencer
RUN cargo build --release -p prediction-market -p synddb-sequencer

# Runtime stage
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    curl \
    sqlite3 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /app/target/release/prediction-market /usr/local/bin/
COPY --from=builder /app/target/release/synddb-sequencer /usr/local/bin/

ENTRYPOINT ["prediction-market"]
