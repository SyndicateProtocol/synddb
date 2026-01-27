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
