# CPU-accelerated SP1 proof generation service with AVX512
#
# Build: docker build -t proof-service .
# Run:   docker run -p 8080:8080 proof-service

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

# Build the proof service with AVX512 acceleration
# See: https://docs.succinct.xyz/docs/sp1/generating-proofs/hardware-acceleration
WORKDIR /app/services/proof-service
ENV RUSTFLAGS="-C target-cpu=x86-64-v4 -C target-feature=+avx512f"
RUN cargo build --release

# Runtime stage - lightweight Debian image (no CUDA needed for CPU mode)
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy the built binary
COPY --from=builder /app/services/proof-service/target/release/proof-service /app/proof-service

# SP1 prover configuration
# Using "local" for CPU-based proving with AVX512 acceleration
ENV SP1_PROVER=local
ENV BIND_ADDRESS=0.0.0.0:8080
ENV RUST_LOG=info

EXPOSE 8080

CMD ["/app/proof-service"]
