# Build stage for Rust components
FROM rust:1.91-bookworm AS rust-builder

WORKDIR /app

# Copy all workspace members needed for cargo to resolve
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY examples ./examples
COPY tests ./tests

# Build the sequencer and FFI library
RUN cargo build --release -p synddb-sequencer && \
    cargo build --release -p synddb-client --features ffi

# Build the validator - it's standalone but references main crates
WORKDIR /app/examples/price-oracle/validator
RUN cargo build --release

# Python application stage
FROM python:3.12-slim-bookworm

RUN apt-get update && apt-get install -y \
    ca-certificates \
    curl \
    sqlite3 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy Rust binaries
COPY --from=rust-builder /app/target/release/synddb-sequencer /usr/local/bin/
COPY --from=rust-builder /app/examples/price-oracle/validator/target/release/price-oracle-validator /usr/local/bin/

# Copy FFI library for Python bindings
COPY --from=rust-builder /app/target/release/libsynddb_client.so /usr/local/lib/

# Copy Python application
COPY examples/price-oracle/app /app/app

# Copy Python SDK bindings
COPY crates/synddb-client/bindings/python/synddb.py /app/synddb.py

# Install Python dependencies
RUN pip install --no-cache-dir -r /app/app/requirements.txt

# Set up library path and PYTHONPATH
ENV LD_LIBRARY_PATH=/usr/local/lib
ENV PYTHONPATH=/app

ENTRYPOINT ["python", "-m", "app.main"]
