# Build stage for Rust FFI library
FROM rust:1.92-trixie AS rust-builder

# Install clang for bindgen (needed for rusqlite bundled feature)
RUN apt-get update && apt-get install -y clang && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy all workspace members needed for cargo to resolve
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY examples ./examples
COPY tests ./tests

# Build only the FFI library for Python bindings
RUN cargo build --release -p synddb-client --features ffi

# Python application stage
FROM python:3.12-slim-trixie

RUN apt-get update && apt-get install -y \
    ca-certificates \
    curl \
    sqlite3 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Create data directory for SQLite database
RUN mkdir -p /data

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

# Confidential Space labels
# allow_env_override: Environment variables that can be set by the VM operator
# Core: RUST_LOG, DATABASE_PATH, SEQUENCER_URL
# Daemon: FETCH_INTERVAL, ASSETS
# API Keys: COINGECKO_API_KEY, CMC_API_KEY
# Chain Monitor: CHAIN_MONITOR_ENABLED, CHAIN_MONITOR_RPC_URL, CHAIN_MONITOR_CONTRACT, CHAIN_MONITOR_POLL
# Bridge: BRIDGE_CONTRACT_ADDRESS
# TEE Bootstrap: ENABLE_KEY_BOOTSTRAP, TEE_KEY_MANAGER_CONTRACT_ADDRESS, BOOTSTRAP_RPC_URL, BOOTSTRAP_CHAIN_ID, PROOF_SERVICE_URL, ATTESTATION_AUDIENCE, RELAYER_URL, IMAGE_SIGNATURE
LABEL "tee.launch_policy.allow_env_override"="RUST_LOG,DATABASE_PATH,SEQUENCER_URL,FETCH_INTERVAL,ASSETS,COINGECKO_API_KEY,CMC_API_KEY,CHAIN_MONITOR_ENABLED,CHAIN_MONITOR_RPC_URL,CHAIN_MONITOR_CONTRACT,CHAIN_MONITOR_POLL,BRIDGE_CONTRACT_ADDRESS,ENABLE_KEY_BOOTSTRAP,TEE_KEY_MANAGER_CONTRACT_ADDRESS,BOOTSTRAP_RPC_URL,BOOTSTRAP_CHAIN_ID,PROOF_SERVICE_URL,ATTESTATION_AUDIENCE,RELAYER_URL,IMAGE_SIGNATURE"
LABEL "tee.launch_policy.log_redirect"="always"
LABEL "tee.launch_policy.allow_ssh"="never"

ENTRYPOINT ["python", "-m", "app.main"]
CMD ["run-daemon"]
