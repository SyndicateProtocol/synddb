# SyndDB Development Commands
#
# Install just: https://github.com/casey/just
#   brew install just
#   cargo install just
#
# Usage:
#   just                    # Show available commands
#   just dev                # Start full local environment
#   just test               # Run all tests
#   just check              # Run all lints
#   just fmt                # Format code
#
# Modules:
#   just contracts::build   # Build Solidity contracts
#   just contracts::test    # Run contract tests
#   just examples::price-oracle       # Run price oracle example
#   just examples::prediction-market  # Run prediction market example
# ============================================================================
# Settings
# ============================================================================
# Use bash with strict mode for all recipes

set shell := ["bash", "-euo", "pipefail", "-c"]

# Load .env if present (for optional local overrides)

set dotenv-load := true

# Export all variables as environment variables

set export := true

# ============================================================================
# Modules
# ============================================================================

mod contracts 'contracts/mod.just'
mod examples 'examples/mod.just'

# ============================================================================
# Shared Configuration (Single Source of Truth)
# ============================================================================
#
# All local development defaults are defined here. No .env file needed!
# To override any value, create a .env file (gitignored) with your custom values.
#
# ============================================================================
# Well-known Anvil addresses and keys (DO NOT USE IN PRODUCTION)
# These are deterministic test accounts - safe to commit

anvil_key_0 := "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
anvil_key_1 := "59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d"
anvil_address_0 := "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"
anvil_address_1 := "0x70997970C51812dc3A010C7d01b50e0d17dc79C8"

# Sequencer public key (derived from anvil_key_0)

sequencer_pubkey := "8318535b54105d4a7aae60c08fc45f9687181b4fdfc625bd1a753fa7397fed753547f11ca8696646f2f3acb08e31016afac23e630c5d11f59f61fef57b0d2aa5"

# Deterministic contract addresses (from deploy-local.sh with Anvil account 0)

bridge_address := "0xe7f1725E7734CE288F8367e1Bb143E90bb3F0512"
weth_address := "0x5FbDB2315678afecb367f032d93F642f64180aa3"
price_oracle_address := "0xCf7Ed3AccA5a467e9e704C703E8D87F634fB0Fc9"

# Network configuration

anvil_rpc_url := "http://127.0.0.1:8545"
anvil_chain_id := "31337"

# Default ports

anvil_port := "8545"
sequencer_port := "8433"
validator_port := "8080"

# Data directories

data_dir := "./data"

# Logging (can override with RUST_LOG env var)

rust_log := "info"

# ============================================================================
# Default & Help
# ============================================================================

# Show available commands (grouped)
default:
    @just --list --unsorted

# ============================================================================
# Local Development
# ============================================================================

# Start local dev environment (Anvil + contracts + sequencer)
[group('dev')]
dev:
    ./scripts/dev-env.sh

# Start full dev environment (Anvil + contracts + sequencer + validator)
[group('dev')]
dev-full:
    ./scripts/dev-env.sh --validator

# Watch mode - rebuild on file changes
[group('dev')]
watch:
    cargo watch -x 'check --workspace'

# Deploy contracts to local Anvil (starts Anvil if not running)
[group('dev')]
deploy:
    ./scripts/deploy-local.sh

# Deploy with fresh Anvil (kills existing)
[group('dev')]
deploy-fresh:
    ./scripts/deploy-local.sh --reset

# ============================================================================
# Components
# ============================================================================

# Start Anvil only
[group('components')]
anvil:
    anvil --port {{ anvil_port }}

# Run sequencer with local defaults
[group('components')]
sequencer:
    mkdir -p {{ data_dir }}
    SIGNING_KEY={{ anvil_key_0 }} \
    BIND_ADDRESS=127.0.0.1:{{ sequencer_port }} \
    DATABASE_PATH={{ data_dir }}/sequencer.db \
    cargo run -p synddb-sequencer --release

# Run validator with local defaults
[group('components')]
validator:
    mkdir -p {{ data_dir }}
    SEQUENCER_PUBKEY={{ sequencer_pubkey }} \
    SEQUENCER_URL=http://127.0.0.1:{{ sequencer_port }} \
    DATABASE_PATH={{ data_dir }}/validator.db \
    STATE_DB_PATH={{ data_dir }}/validator_state.db \
    PENDING_CHANGESETS_DB_PATH={{ data_dir }}/pending_changesets.db \
    cargo run -p synddb-validator --release

# Run validator with bridge signer enabled
[group('components')]
validator-bridge:
    mkdir -p {{ data_dir }}
    SEQUENCER_PUBKEY={{ sequencer_pubkey }} \
    SEQUENCER_URL=http://127.0.0.1:{{ sequencer_port }} \
    DATABASE_PATH={{ data_dir }}/validator.db \
    STATE_DB_PATH={{ data_dir }}/validator_state.db \
    PENDING_CHANGESETS_DB_PATH={{ data_dir }}/pending_changesets.db \
    BRIDGE_SIGNER=true \
    BRIDGE_CONTRACT={{ bridge_address }} \
    BRIDGE_CHAIN_ID=31337 \
    BRIDGE_SIGNING_KEY={{ anvil_key_0 }} \
    cargo run -p synddb-validator --release

# ============================================================================
# Building
# ============================================================================

# Build all crates (debug)
[group('build')]
build:
    cargo build --workspace

# Build all crates (release)
[group('build')]
build-release:
    cargo build --workspace --release

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
# Linting (CI uses these individually, `just check` runs all)
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

# Run all lints (for local development)
[group('lint')]
check: lint-toml lint-deps lint-fmt lint
    @echo "All checks passed!"

# ============================================================================
# Formatting
# ============================================================================

# Format all code (Rust, TOML, Solidity)
[group('format')]
fmt:
    cargo +nightly fmt --all
    taplo fmt "**/Cargo.toml"
    cd contracts && forge fmt

# Fix all auto-fixable issues
[group('format')]
fix:
    taplo fmt "**/Cargo.toml"
    cargo +nightly fmt --all
    cargo clippy --workspace --all-targets --all-features --fix --allow-dirty --allow-staged

# ============================================================================
# Git Hooks
# ============================================================================

# Install git hooks (lefthook)
[group('hooks')]
hooks-install:
    lefthook install

# Uninstall git hooks
[group('hooks')]
hooks-uninstall:
    lefthook uninstall

# Run pre-commit hooks manually
[group('hooks')]
hooks-run:
    lefthook run pre-commit

# ============================================================================
# Cleanup
# ============================================================================

# Clean build artifacts
[confirm('This will remove all build artifacts. Continue?')]
[group('cleanup')]
clean:
    cargo clean
    cd contracts && forge clean

# Clean local development data
[confirm('This will remove all local data in ./data and ./.synddb. Continue?')]
[group('cleanup')]
clean-data:
    rm -rf ./data
    rm -rf ./.synddb

# Clean everything
[confirm('This will remove ALL build artifacts and data. Continue?')]
[group('cleanup')]
clean-all: clean clean-data examples::clean

# Kill any running Anvil processes
[group('cleanup')]
kill-anvil:
    pkill -f "anvil" || true

# ============================================================================
# Documentation
# ============================================================================

# Build and open docs in browser
[group('docs')]
docs:
    cargo doc --workspace --all-features --no-deps --open

# ============================================================================
# Dependencies
# ============================================================================

# Show outdated dependencies
[group('deps')]
outdated:
    cargo outdated -R

# Run security audit
[group('deps')]
audit:
    cargo audit

# ============================================================================
# Release (disabled until ready for production)
# ============================================================================

# Generate changelog from conventional commits
[group('release')]
changelog:
    git cliff --unreleased

# Dry-run release (shows what would happen)
[group('release')]
release-dry version:
    cargo release {{ version }} --no-publish --no-tag --no-push

# Execute release (requires cargo-release: cargo install cargo-release)
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

# ============================================================================
# Docker
# ============================================================================

# Run with Docker Compose
[group('docker')]
docker-up:
    docker compose up --build

# Run with Docker Compose (detached)
[group('docker')]
docker-detached:
    docker compose up --build -d

# Stop Docker Compose
[group('docker')]
docker-down:
    docker compose down -v
