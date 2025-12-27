# Reproducible Builds

This directory contains Dockerfiles for building bit-for-bit reproducible container images, designed for verification in Google Cloud Confidential Space.

## Quick Start

```bash
# Build reproducible images
just repro-sequencer
just repro-validator

# Or build both
just repro-all

# Verify image digests
just repro-verify
```

## Why Reproducible Builds?

In Confidential Space, the container image hash is part of the attestation. For this to be meaningful:

1. Anyone can rebuild from source and get the **exact same image hash**
2. The image hash proves the code running inside the TEE

This requires eliminating all sources of non-determinism in the build process.

## Build Targets

Each Dockerfile provides two targets:

| Target | Base Image | Use Case |
|--------|------------|----------|
| `release` (default) | `gcr.io/distroless/cc-debian12` | Production / TEE deployment |
| `debug` | `debian:bookworm-slim` | Local debugging (has shell) |

```bash
# Production (distroless, no shell)
just repro-sequencer

# Debug (debian, has shell for troubleshooting)
just repro-sequencer-debug
```

## Reproducibility Features

The Dockerfiles implement these reproducibility measures:

### 1. Pinned Base Images (by digest)

```dockerfile
ARG RUST_IMAGE_DIGEST=sha256:9676d0547a...
FROM rust@${RUST_IMAGE_DIGEST}
```

Tags like `rust:1.92-bookworm` can change over time. Digests are immutable.

### 2. Deterministic Timestamps

```dockerfile
ENV SOURCE_DATE_EPOCH=0
```

All timestamps in the build are set to Unix epoch (1970-01-01), eliminating time-based variation.

### 3. Path Normalization

```dockerfile
ENV RUSTFLAGS="--remap-path-prefix=/app=/build --remap-path-prefix=/usr/local/cargo=/cargo"
```

Absolute paths embedded in binaries (debug info, panic messages) are normalized to `/build` and `/cargo`, regardless of where the build actually runs.

### 4. Locked Dependencies

```dockerfile
RUN cargo build --release --locked -p synddb-sequencer
```

The `--locked` flag ensures `Cargo.lock` is respected exactly, preventing dependency drift.

### 5. Disabled Incremental Compilation

```dockerfile
ENV CARGO_INCREMENTAL=0
```

Incremental compilation can produce non-deterministic output.

### 6. Stripped Binaries

```dockerfile
RUN strip --strip-all /app/target/release/synddb-sequencer
```

Stripping removes variable metadata (build IDs, timestamps) from the binary.

## Verification

To verify a build is reproducible:

```bash
# Build on machine A
docker build -f docker/reproducible/sequencer.Dockerfile \
    --platform linux/amd64 \
    -t synddb-sequencer:test .

# Get the image ID
docker inspect synddb-sequencer:test --format='{{.Id}}'
# Output: sha256:abc123...

# Build on machine B (same source, same commit)
# Should produce the exact same sha256:abc123...
```

## Updating Base Images

When Rust or Debian releases updates, you'll need to update the pinned digests:

```bash
# Get the new digest
docker pull rust:1.92-bookworm
docker inspect rust:1.92-bookworm --format='{{index .RepoDigests 0}}'

docker pull debian:bookworm-slim
docker inspect debian:bookworm-slim --format='{{index .RepoDigests 0}}'

docker pull gcr.io/distroless/cc-debian12
docker inspect gcr.io/distroless/cc-debian12 --format='{{index .RepoDigests 0}}'
```

Then update the `ARG` values in the Dockerfiles.

## Confidential Space Labels

The release images include Confidential Space workload labels:

```dockerfile
LABEL "tee.launch_policy.allow_env_override"="RUST_LOG,BIND_ADDRESS,..."
LABEL "tee.launch_policy.log_redirect"="always"
```

These declare which environment variables operators can override without changing the attestation.

## Limitations

- **Build cache**: Docker layer caching can affect reproducibility if layers are reused from different builds. Use `--no-cache` for guaranteed reproducibility.
- **BuildKit**: Different BuildKit versions may produce different layer hashes. Pin your Docker/BuildKit version in CI.
- **Architecture**: Builds are pinned to `linux/amd64` for Confidential Space VMs.

## CI Integration

For CI verification, add a job that:

1. Builds the image twice (with `--no-cache`)
2. Compares the image digests
3. Fails if they differ

```yaml
- name: Verify reproducible build
  run: |
    docker build --no-cache -f docker/reproducible/sequencer.Dockerfile \
        --platform linux/amd64 -t test1 .
    docker build --no-cache -f docker/reproducible/sequencer.Dockerfile \
        --platform linux/amd64 -t test2 .

    HASH1=$(docker inspect test1 --format='{{.Id}}')
    HASH2=$(docker inspect test2 --format='{{.Id}}')

    if [ "$HASH1" != "$HASH2" ]; then
        echo "Build is not reproducible!"
        exit 1
    fi
```
