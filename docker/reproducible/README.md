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
| `release` (default) | `gcr.io/distroless/cc-debian13` | Production / TEE deployment |
| `debug` | `debian:trixie-slim` | Local debugging (has shell) |

```bash
# Production (distroless, no shell)
just repro-sequencer

# Debug (debian, has shell for troubleshooting)
just repro-sequencer-debug
```

## Reproducibility Configuration

Reproducible builds require eliminating all sources of non-determinism. This section documents every flag and why it's necessary.

### Rust/Cargo Configuration

| Setting | Why It's Needed |
|---------|-----------------|
| `CARGO_BUILD_JOBS=1` | Parallel compilation produces non-deterministic ordering. Sequential builds are deterministic. |
| `CARGO_INCREMENTAL=0` | Incremental compilation caches can vary between builds. |
| `CARGO_HOME=/cargo` | Fixes the cargo registry path. Different paths would embed differently in debug info. |
| `--locked` | Ensures `Cargo.lock` is respected exactly, preventing dependency drift. |
| `--remap-path-prefix` | Normalizes absolute paths in binaries. Without this, `/home/user/project` would differ from `/app`. |

**Cargo Profile** (`Cargo.toml`):
```toml
[profile.reproducible]
inherits = "release"
codegen-units = 1  # Parallel codegen is non-deterministic
lto = true         # Single LTO pass is more deterministic
panic = "abort"    # Removes unwinding tables that can vary
```

### Docker/BuildKit Configuration

| Setting | Why It's Needed |
|---------|-----------------|
| `--provenance=false` | Provenance attestations embed build timestamps and machine info. **Not reproducible by design.** |
| `--sbom=false` | SBOM attestations include variable metadata. |
| `--no-cache` | Layer caching can reuse non-reproducible intermediate results. |
| `rewrite-timestamp=true` | Rewrites all file timestamps in image layers to `SOURCE_DATE_EPOCH`. |
| `SOURCE_DATE_EPOCH=0` | Standard for reproducible builds. Sets all timestamps to Unix epoch (1970-01-01). |
| Pinned BuildKit version | Different versions may hash layers differently. Version is defined in `buildkit.version` (shared between CI and local builds). |
| Pinned base images by digest | Tags like `rust:1.92` can change. Digests (`sha256:...`) are immutable. |

### Dockerfile Steps

| Step | Why It's Needed |
|------|-----------------|
| `find /app -type f -exec touch -d "@0" {} +` | Normalizes source file timestamps before compilation. |
| `strip --strip-all` | Removes variable metadata (build IDs, timestamps) from binaries. |
| `touch -d "@0" <binary>` | Ensures final binary has deterministic timestamp. |

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
docker pull rust:1.92-trixie
docker inspect rust:1.92-trixie --format='{{index .RepoDigests 0}}'

docker pull debian:trixie-slim
docker inspect debian:trixie-slim --format='{{index .RepoDigests 0}}'

docker pull gcr.io/distroless/cc-debian13
docker inspect gcr.io/distroless/cc-debian13 --format='{{index .RepoDigests 0}}'
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
3. **Uses pinned BuildKit**: Reads version from `buildkit.version` to ensure consistency with local builds
4. **Cosign signing**: Signs images with keyless OIDC for supply chain security

```yaml
# Read version from shared file
- run: echo "version=$(cat docker/reproducible/buildkit.version)" >> $GITHUB_OUTPUT
  id: buildkit

- uses: docker/setup-buildx-action@v3
  with:
    driver-opts: |
      image=moby/buildkit:${{ steps.buildkit.outputs.version }}

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
