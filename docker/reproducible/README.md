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
RUN cargo build --profile reproducible --locked -p synddb-sequencer
```

The `--locked` flag ensures `Cargo.lock` is respected exactly, preventing any dependency drift.

### 5. Sequential Compilation

```dockerfile
ENV CARGO_BUILD_JOBS=1
```

Parallel compilation can produce different orderings. Single-threaded builds are deterministic.

### 6. Reproducible Cargo Profile

The workspace defines a `[profile.reproducible]` in `Cargo.toml`:

```toml
[profile.reproducible]
inherits = "release"
codegen-units = 1  # Single codegen unit for deterministic compilation
lto = true         # Link-time optimization in one pass
panic = "abort"    # Simpler binary without unwinding tables
```

### 7. Disabled Incremental Compilation

```dockerfile
ENV CARGO_INCREMENTAL=0
```

Incremental compilation can produce non-deterministic output.

### 8. Stripped Binaries

```dockerfile
RUN strip --strip-all /app/target/reproducible/synddb-sequencer
```

Stripping removes variable metadata (build IDs, timestamps) from the binary.

### 9. Fixed CARGO_HOME

```dockerfile
ENV CARGO_HOME=/cargo
```

Ensures cargo registry and caches use a consistent path across builds.

### 10. Normalized File Timestamps

```dockerfile
RUN find /app -type f -exec touch -d "@0" {} +
RUN touch -d "@0" /app/target/reproducible/synddb-sequencer
```

Normalizes all source file and binary timestamps to epoch.

## BuildKit Configuration (Critical)

The following BuildKit settings are **essential** for reproducibility:

### 1. Disable Provenance Attestations

```bash
docker buildx build --provenance=false
```

Provenance attestations include non-reproducible metadata (build timestamps, builder info). Disable them.

### 2. Disable SBOM Attestations

```bash
docker buildx build --sbom=false
```

SBOM (Software Bill of Materials) attestations may also include variable data.

### 3. Rewrite Timestamps

```bash
docker buildx build --output type=docker,rewrite-timestamp=true
```

This BuildKit v0.13+ feature rewrites all file timestamps in the final image to match `SOURCE_DATE_EPOCH`.

### 4. Pass SOURCE_DATE_EPOCH

```bash
SOURCE_DATE_EPOCH=0 docker buildx build --build-arg SOURCE_DATE_EPOCH=0
```

Pass as both an environment variable and a build argument for complete coverage.

### 5. Pin BuildKit Version

```yaml
- uses: docker/setup-buildx-action@v3
  with:
    driver-opts: |
      image=moby/buildkit:v0.18.2
```

Different BuildKit versions may produce different layer hashes.

### Complete Build Command

```bash
SOURCE_DATE_EPOCH=0 docker buildx build \
    --no-cache \
    --provenance=false \
    --sbom=false \
    --build-arg SOURCE_DATE_EPOCH=0 \
    --platform linux/amd64 \
    --output type=docker,rewrite-timestamp=true \
    -f docker/reproducible/sequencer.Dockerfile \
    -t synddb-sequencer:reproducible \
    .
```

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

See `.github/workflows/reproducible-builds.yml` for the full CI workflow. Key features:

1. **Same-machine verification**: Builds twice on the same runner, compares hashes
2. **Cross-machine verification**: Builds on Ubuntu 22.04 and 24.04, compares hashes across runners
3. **Uses pinned BuildKit**: Ensures consistent BuildKit version across all builds

```yaml
- uses: docker/setup-buildx-action@v3
  with:
    driver-opts: |
      image=moby/buildkit:v0.18.2

- name: Build image
  env:
    SOURCE_DATE_EPOCH: 0
  run: |
    docker buildx build \
      --no-cache \
      --provenance=false \
      --sbom=false \
      --build-arg SOURCE_DATE_EPOCH=0 \
      --platform linux/amd64 \
      --output type=docker,rewrite-timestamp=true \
      -f docker/reproducible/sequencer.Dockerfile \
      -t sequencer:build1 \
      .
```

## References

- [Docker Reproducible Builds](https://docs.docker.com/build/ci/github-actions/reproducible-builds/)
- [BuildKit Reproducibility](https://github.com/moby/buildkit/blob/master/docs/build-repro.md)
- [Rust Reproducible Builds](https://github.com/rust-lang/cargo/issues/5505)
- [SOURCE_DATE_EPOCH](https://reproducible-builds.org/docs/source-date-epoch/)
