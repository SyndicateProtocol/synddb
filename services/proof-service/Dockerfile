# GPU-accelerated SP1 proof generation service
#
# Build: docker build -t proof-service .
# Run:   docker run --gpus all -p 8080:8080 proof-service

# Build stage
FROM rust:1.92-bookworm AS builder

# Install SP1 toolchain dependencies
# Go 1.22+ and Clang are required for native-gnark feature (avoids Docker-in-Docker)
RUN apt-get update && apt-get install -y \
    cmake \
    pkg-config \
    libssl-dev \
    clang \
    && rm -rf /var/lib/apt/lists/*

# Install Go 1.25 (required for native-gnark)
RUN curl -LO https://go.dev/dl/go1.25.5.linux-amd64.tar.gz && \
    tar -C /usr/local -xzf go1.25.5.linux-amd64.tar.gz && \
    rm go1.25.5.linux-amd64.tar.gz
ENV PATH="/usr/local/go/bin:${PATH}"

# Install SP1 toolchain
RUN curl -L https://sp1.succinct.xyz | bash && \
    ~/.sp1/bin/sp1up

WORKDIR /app

# Copy the entire repo context (needed for workspace dependencies)
COPY . .

# Build the proof service
WORKDIR /app/services/proof-service
RUN cargo build --release

# Runtime stage with CUDA support
# Using CUDA 12.5.1 to meet SP1's >= 12.5.1 recommendation
# Cloud Run provides driver 535 (CUDA 12.2), so we need forward compatibility
FROM nvidia/cuda:12.5.1-runtime-ubuntu22.04

RUN apt-get update && apt-get install -y \
    ca-certificates \
    cuda-compat-12-5 \
    && rm -rf /var/lib/apt/lists/*

# Use forward compatibility libraries for Cloud Run's older driver (535/CUDA 12.2)
ENV LD_LIBRARY_PATH=/usr/local/cuda-12.5/compat:${LD_LIBRARY_PATH}

WORKDIR /app

# Copy the built binary
COPY --from=builder /app/services/proof-service/target/release/proof-service /app/proof-service

# SP1 prover configuration
# Set to "cuda" for GPU acceleration, "local" for CPU, or "network" for SP1 Network
ENV SP1_PROVER=cuda
ENV BIND_ADDRESS=0.0.0.0:8080
ENV RUST_LOG=info

EXPOSE 8080

CMD ["/app/proof-service"]
