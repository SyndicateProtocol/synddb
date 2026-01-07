# SyndDB CI Commands
#
# This justfile contains commands used by CI workflows.
# For local development, use cargo/forge commands directly.
#
# Install just: https://github.com/casey/just
#   brew install just
#   cargo install just
#
# Usage:
#   just                    # Show available commands
#   just test               # Run all tests
#   just check              # Run all lints
#
# ============================================================================
# Settings
# ============================================================================

set shell := ["bash", "-euo", "pipefail", "-c"]
set dotenv-load := true
set dotenv-filename := ".env.defaults"
set export := true

# ============================================================================
# Configuration
# ============================================================================
# Variables loaded from .env.defaults for CI test commands

anvil_key_0 := env_var('ANVIL_KEY_0')

# ============================================================================
# Default & Help
# ============================================================================

# Show available commands
default:
    @just --list --unsorted

# ============================================================================
# Testing
# ============================================================================

# Run unit tests
[group('test')]
test:
    cargo nextest run --workspace --all-features --exclude synddb-e2e --exclude synddb-e2e-gcs

# Run tests with output
[group('test')]
test-verbose:
    cargo nextest run --workspace --all-features --exclude synddb-e2e --exclude synddb-e2e-gcs --no-capture

# Run stress test (starts sequencer automatically)
[group('test')]
test-stress:
    #!/usr/bin/env bash
    set -euxo pipefail
    cargo build --package synddb-benchmark --bin session-stress-test --release
    cargo build --package synddb-sequencer --release
    SIGNING_KEY={{ anvil_key_0 }} ./target/release/synddb-sequencer &
    SEQUENCER_PID=$!
    trap "kill $SEQUENCER_PID 2>/dev/null || true" EXIT
    for i in {1..30}; do
        if curl -s http://localhost:8433/health > /dev/null 2>&1; then
            echo "Sequencer is healthy"
            break
        fi
        echo "Waiting for sequencer... ($i/30)"
        sleep 1
    done
    ./target/release/session-stress-test --duration ${STRESS_TEST_DURATION:-15}

# Run client integration tests (starts sequencer automatically)
[group('test')]
test-integration:
    #!/usr/bin/env bash
    set -euxo pipefail
    cargo build --package synddb-sequencer --release
    SIGNING_KEY={{ anvil_key_0 }} ./target/release/synddb-sequencer &
    SEQUENCER_PID=$!
    trap "kill $SEQUENCER_PID 2>/dev/null || true" EXIT
    for i in {1..30}; do
        if curl -s http://localhost:8433/health > /dev/null 2>&1; then
            echo "Sequencer is healthy"
            break
        fi
        echo "Waiting for sequencer... ($i/30)"
        sleep 1
    done
    cargo test -p synddb-client --lib -- --ignored --skip attestation

# Run SQLite fuzzer (set PROPTEST_CASES for more iterations)
[group('test')]
fuzz:
    PROPTEST_CASES=${PROPTEST_CASES:-256} cargo test -p synddb-fuzzer --release

# Run E2E pipeline fuzzer (set PROPTEST_CASES for more iterations)
[group('test')]
fuzz-e2e:
    PROPTEST_CASES=${PROPTEST_CASES:-100} cargo test -p synddb-e2e-fuzzer --release

# Run E2E stress tests (set PROPTEST_CASES for more iterations)
[group('test')]
fuzz-e2e-stress:
    PROPTEST_CASES=${PROPTEST_CASES:-10} cargo test -p synddb-e2e-fuzzer --release -- --ignored

# ============================================================================
# Linting
# ============================================================================

# Run Clippy lints
[group('lint')]
lint:
    cargo clippy --workspace --all-targets --all-features

# Check TOML formatting
[group('lint')]
lint-toml:
    taplo lint "**/Cargo.toml"
    taplo fmt --check "**/Cargo.toml"

# Check for unused dependencies
[group('lint')]
lint-deps:
    cargo machete

# Check Rust formatting
[group('lint')]
lint-fmt:
    cargo +nightly fmt --all --check

# Check documentation builds without warnings
[group('lint')]
lint-docs:
    cargo doc --workspace --all-features --no-deps

# Run all lints
[group('lint')]
check: lint-toml lint-deps lint-fmt lint
    @echo "All checks passed!"

# ============================================================================
# Reproducible Builds (for Confidential Space / TEE verification)
# ============================================================================
# BuildKit version pinned for reproducibility

buildkit_version := "moby/buildkit:" + trim(read("docker/reproducible/buildkit.version"))
buildkit_builder := "synddb-repro"

# Set up BuildKit builder with pinned version
[group('reproducible')]
repro-setup:
    #!/usr/bin/env bash
    set -euo pipefail
    if docker buildx inspect {{ buildkit_builder }} >/dev/null 2>&1; then
        echo "Builder '{{ buildkit_builder }}' already exists"
    else
        echo "Creating builder '{{ buildkit_builder }}' with {{ buildkit_version }}"
        docker buildx create --name {{ buildkit_builder }} --driver docker-container --driver-opt image={{ buildkit_version }}
    fi
    docker buildx inspect {{ buildkit_builder }} --bootstrap

# Build reproducible sequencer image
[group('reproducible')]
repro-sequencer: repro-setup
    SOURCE_DATE_EPOCH=0 docker buildx build \
        --builder {{ buildkit_builder }} \
        --no-cache \
        --provenance=false \
        --sbom=false \
        --build-arg SOURCE_DATE_EPOCH=0 \
        --platform linux/amd64 \
        --output type=docker,rewrite-timestamp=true \
        -f docker/reproducible/sequencer.Dockerfile \
        -t synddb-sequencer:reproducible .

# Build reproducible validator image
[group('reproducible')]
repro-validator: repro-setup
    SOURCE_DATE_EPOCH=0 docker buildx build \
        --builder {{ buildkit_builder }} \
        --no-cache \
        --provenance=false \
        --sbom=false \
        --build-arg SOURCE_DATE_EPOCH=0 \
        --platform linux/amd64 \
        --output type=docker,rewrite-timestamp=true \
        -f docker/reproducible/validator.Dockerfile \
        -t synddb-validator:reproducible .

# Build all reproducible images
[group('reproducible')]
repro-all: repro-sequencer repro-validator

# Base images for reproducible builds
repro_rust_image := "rust:1.92-trixie"
repro_debian_image := "debian:trixie-slim"
repro_distroless_image := "gcr.io/distroless/cc-debian13"

# Check for newer base image digests
[group('reproducible')]
check-digests:
    #!/usr/bin/env bash
    set -euo pipefail

    echo "=== Checking Base Image Digests ==="
    echo ""
    echo "Fetching latest digests from registries..."
    echo ""

    RUST_LATEST=$(docker manifest inspect {{ repro_rust_image }} -v 2>/dev/null | jq -r '.Descriptor.digest // .digest' 2>/dev/null || echo "failed to fetch")
    DEBIAN_LATEST=$(docker manifest inspect {{ repro_debian_image }} -v 2>/dev/null | jq -r '.Descriptor.digest // .digest' 2>/dev/null || echo "failed to fetch")
    DISTROLESS_LATEST=$(docker manifest inspect {{ repro_distroless_image }} -v 2>/dev/null | jq -r '.Descriptor.digest // .digest' 2>/dev/null || echo "failed to fetch")

    echo "{{ repro_rust_image }}"
    echo "  Latest: $RUST_LATEST"
    CURRENT=$(grep 'ARG RUST_IMAGE_DIGEST=' docker/reproducible/sequencer.Dockerfile | head -1 | cut -d= -f2)
    echo "  Current: $CURRENT"
    [ "$RUST_LATEST" = "$CURRENT" ] && echo "  Up to date" || echo "  Update available"
    echo ""

    echo "{{ repro_debian_image }}"
    echo "  Latest: $DEBIAN_LATEST"
    CURRENT=$(grep 'ARG DEBIAN_IMAGE_DIGEST=' docker/reproducible/sequencer.Dockerfile | head -1 | cut -d= -f2)
    echo "  Current: $CURRENT"
    [ "$DEBIAN_LATEST" = "$CURRENT" ] && echo "  Up to date" || echo "  Update available"
    echo ""

    echo "{{ repro_distroless_image }}"
    echo "  Latest: $DISTROLESS_LATEST"
    CURRENT=$(grep 'ARG DISTROLESS_IMAGE_DIGEST=' docker/reproducible/sequencer.Dockerfile | head -1 | cut -d= -f2)
    echo "  Current: $CURRENT"
    [ "$DISTROLESS_LATEST" = "$CURRENT" ] && echo "  Up to date" || echo "  Update available"

# Update base image digests in reproducible Dockerfiles
[confirm("Update all digest pins in reproducible Dockerfiles?")]
[group('reproducible')]
update-digests:
    #!/usr/bin/env bash
    set -euo pipefail

    echo "=== Updating Base Image Digests ==="
    echo ""

    RUST=$(docker manifest inspect {{ repro_rust_image }} -v 2>/dev/null | jq -r '.Descriptor.digest // .digest')
    DEBIAN=$(docker manifest inspect {{ repro_debian_image }} -v 2>/dev/null | jq -r '.Descriptor.digest // .digest')
    DISTROLESS=$(docker manifest inspect {{ repro_distroless_image }} -v 2>/dev/null | jq -r '.Descriptor.digest // .digest')

    if [ -z "$RUST" ] || [ -z "$DEBIAN" ] || [ -z "$DISTROLESS" ]; then
        echo "Error: Failed to fetch one or more digests"
        exit 1
    fi

    echo "New digests:"
    echo "  Rust:       $RUST"
    echo "  Debian:     $DEBIAN"
    echo "  Distroless: $DISTROLESS"
    echo ""

    # Detect OS for sed compatibility
    if [[ "$OSTYPE" == "darwin"* ]]; then
        SED_INPLACE="sed -i ''"
    else
        SED_INPLACE="sed -i"
    fi

    for f in docker/reproducible/*.Dockerfile; do
        echo "Updating $f..."
        $SED_INPLACE "s|ARG RUST_IMAGE_DIGEST=.*|ARG RUST_IMAGE_DIGEST=${RUST}|" "$f"
        $SED_INPLACE "s|ARG DEBIAN_IMAGE_DIGEST=.*|ARG DEBIAN_IMAGE_DIGEST=${DEBIAN}|" "$f"
        $SED_INPLACE "s|ARG DISTROLESS_IMAGE_DIGEST=.*|ARG DISTROLESS_IMAGE_DIGEST=${DISTROLESS}|" "$f"
    done

    echo ""
    echo "=== Digests Updated ==="
    echo ""
    echo "Next steps:"
    echo "  1. Review changes: git diff docker/reproducible/"
    echo "  2. Build and test: just repro-all"
    echo "  3. Commit: git add docker/reproducible/ && git commit -m 'chore: update base image digests'"

# ============================================================================
# Release
# ============================================================================

# Generate changelog from conventional commits
[group('release')]
changelog:
    git cliff --unreleased

# Dry-run release (shows what would happen)
[group('release')]
release-dry version:
    cargo release {{ version }} --no-publish --no-tag --no-push

# Execute release (requires cargo-release)
[group('release')]
release version:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ -z "${CI:-}" ] && [ -z "${CARGO_REGISTRY_TOKEN:-}" ]; then
        echo "Error: Not running in CI and no CARGO_REGISTRY_TOKEN set."
        echo "Use 'just release-dry {{ version }}' to test locally."
        exit 1
    fi
    echo "This will publish to crates.io. Press Ctrl+C to cancel, or Enter to continue..."
    read -r
    cargo release {{ version }} --execute
