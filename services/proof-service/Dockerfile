# GPU-accelerated SP1 proof generation service
#
# Build: docker build -t proof-service .
# Run:   docker run --gpus all -p 8080:8080 proof-service

# Build stage
FROM rust:1.83-bookworm AS builder

# Install SP1 toolchain dependencies
RUN apt-get update && apt-get install -y \
    cmake \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

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
FROM nvidia/cuda:12.4.1-runtime-ubuntu22.04

RUN apt-get update && apt-get install -y \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

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
