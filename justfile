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

# Load .env.defaults for local development configuration
# To override values, create .env (gitignored) and source it before running just,
# or use direnv with .envrc

set dotenv-load := true
set dotenv-filename := ".env.defaults"

# Export all variables as environment variables

set export := true

# ============================================================================
# Modules
# ============================================================================

mod contracts 'contracts/mod.just'
mod examples 'examples/mod.just'

# ============================================================================
# Configuration from .env.defaults
# ============================================================================
# All values are loaded from .env.defaults. To override, set environment
# variables before running just, or create .env and source it.
#
# These variables reference the dotenv values for use in recipes:

anvil_key_0 := env_var('ANVIL_KEY_0')
anvil_key_1 := env_var('ANVIL_KEY_1')
anvil_address_0 := env_var('ANVIL_ADDRESS_0')
anvil_address_1 := env_var('ANVIL_ADDRESS_1')
bridge_contract_address := env_var('BRIDGE_CONTRACT_ADDRESS')
weth_contract_address := env_var('WETH_CONTRACT_ADDRESS')
price_oracle_contract_address := env_var('PRICE_ORACLE_CONTRACT_ADDRESS')
tee_key_manager_contract_address := env_var('TEE_KEY_MANAGER_CONTRACT_ADDRESS')
anvil_rpc_url := env_var('ANVIL_RPC_URL')
anvil_chain_id := env_var('ANVIL_CHAIN_ID')
anvil_port := env_var('ANVIL_PORT')
sequencer_port := env_var('SEQUENCER_PORT')
validator_port := env_var('VALIDATOR_PORT')
data_dir := env_var('DATA_DIR')

# ============================================================================
# Default & Help
# ============================================================================

# Show available commands (grouped)
default:
    @just --list --unsorted

# ============================================================================
# Info
# ============================================================================

# Show configured addresses and keys
[group('info')]
info:
    #!/usr/bin/env bash
    set -euo pipefail
    echo "=== Anvil Accounts ==="
    echo "Account 0 (admin/deployer):"
    echo "  Address: {{ anvil_address_0 }}"
    echo "  Key:     {{ anvil_key_0 }}"
    echo ""
    echo "Account 1 (sequencer):"
    echo "  Address: {{ anvil_address_1 }}"
    echo "  Key:     {{ anvil_key_1 }}"
    echo ""
    echo "=== Contract Addresses ==="
    echo "  WETH:         {{ weth_contract_address }}"
    echo "  Bridge:       {{ bridge_contract_address }}"
    echo "  Price Oracle: {{ price_oracle_contract_address }}"
    echo ""
    echo "=== Service URLs ==="
    echo "  Anvil RPC:  {{ anvil_rpc_url }}"
    echo "  Sequencer:  http://127.0.0.1:{{ sequencer_port }}"
    echo "  Validator:  http://127.0.0.1:{{ validator_port }}"

# Fetch live status from services (sequencer, validator, validator-signer, or all)

# Usage: just service-status [service]  (default: all)
[group('info')]
service-status service="all":
    #!/usr/bin/env bash
    set -euo pipefail
    fetch_status() {
        local name="$1" url="$2"
        echo "=== $name ==="
        if curl -s "$url" > /tmp/service_status.json 2>/dev/null; then
            jq '.' /tmp/service_status.json
            rm -f /tmp/service_status.json
        else
            echo "(not running)"
        fi
        echo ""
    }
    case "{{ service }}" in
        all)
            fetch_status "Sequencer Status" "http://127.0.0.1:{{ sequencer_port }}/status"
            fetch_status "Validator Status" "http://127.0.0.1:{{ validator_port }}/status"
            fetch_status "Validator Signer Info" "http://127.0.0.1:8081/info"
            ;;
        sequencer)
            fetch_status "Sequencer Status" "http://127.0.0.1:{{ sequencer_port }}/status"
            ;;
        validator)
            fetch_status "Validator Status" "http://127.0.0.1:{{ validator_port }}/status"
            ;;
        validator-signer)
            fetch_status "Validator Signer Info" "http://127.0.0.1:8081/info"
            ;;
        *)
            echo "Unknown service: {{ service }}"
            echo "Valid services: all, sequencer, validator, validator-signer"
            exit 1
            ;;
    esac

# Alias: Fetch sequencer status
[group('info')]
info-sequencer:
    @just service-status sequencer

# Alias: Fetch validator status
[group('info')]
info-validator:
    @just service-status validator

# Alias: Fetch validator signer info (bridge mode)
[group('info')]
info-validator-signer:
    @just service-status validator-signer

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

# Fund a dynamically generated wallet from Anvil

# Usage: just fund-wallet <address> [amount_eth]
[group('components')]
fund-wallet address amount="1":
    #!/usr/bin/env bash
    set -euo pipefail
    echo "Funding {{ address }} with {{ amount }} ETH..."
    cast send --rpc-url {{ anvil_rpc_url }} \
        --private-key {{ anvil_key_0 }} \
        "{{ address }}" \
        --value "{{ amount }}ether"
    BALANCE=$(cast balance --rpc-url {{ anvil_rpc_url }} "{{ address }}" --ether)
    echo "Done. New balance: $BALANCE ETH"

# Fund service wallets from Anvil (sequencer, validator-signer, or all)

# Usage: just fund-service [service] [amount_eth]  (default: all, 1 ETH)
[group('components')]
fund-service service="all" amount="1":
    #!/usr/bin/env bash
    set -euo pipefail
    fund_one() {
        local name="$1" url="$2" jq_path="$3"
        echo "=== Funding $name ==="
        RESPONSE=$(curl -s "$url" 2>/dev/null) || true
        if [ -z "$RESPONSE" ]; then
            echo "(not running)"
            echo ""
            return 0
        fi
        SIGNER_ADDRESS=$(echo "$RESPONSE" | jq -r "$jq_path" 2>/dev/null) || true
        if [ -z "$SIGNER_ADDRESS" ] || [ "$SIGNER_ADDRESS" = "null" ]; then
            echo "(no signer address available)"
            echo ""
            return 0
        fi
        echo "Address: $SIGNER_ADDRESS"
        cast send --rpc-url {{ anvil_rpc_url }} \
            --private-key {{ anvil_key_0 }} \
            "$SIGNER_ADDRESS" \
            --value "{{ amount }}ether" > /dev/null
        BALANCE=$(cast balance --rpc-url {{ anvil_rpc_url }} "$SIGNER_ADDRESS" --ether)
        echo "Funded {{ amount }} ETH. New balance: $BALANCE ETH"
        echo ""
    }
    case "{{ service }}" in
        all)
            fund_one "Sequencer" "http://127.0.0.1:{{ sequencer_port }}/status" ".signer_address"
            fund_one "Validator Signer" "http://127.0.0.1:8081/info" ".signer"
            ;;
        sequencer)
            fund_one "Sequencer" "http://127.0.0.1:{{ sequencer_port }}/status" ".signer_address"
            ;;
        validator-signer)
            fund_one "Validator Signer" "http://127.0.0.1:8081/info" ".signer"
            ;;
        *)
            echo "Unknown service: {{ service }}"
            echo "Valid services: all, sequencer, validator-signer"
            exit 1
            ;;
    esac

# Alias: Fund the sequencer's dynamically generated wallet
[group('components')]
fund-sequencer amount="1":
    @just fund-service sequencer "{{ amount }}"

# Alias: Fund the validator's bridge signer wallet
[group('components')]
fund-validator amount="1":
    @just fund-service validator-signer "{{ amount }}"

# Register a TEE key with the mock attestation verifier (Anvil only)
# Usage: just register-tee-key <address>

# This uses MockAttestationVerifier which accepts any address without proof verification.
[group('components')]
register-tee-key address:
    #!/usr/bin/env bash
    set -euo pipefail
    echo "Registering TEE key {{ address }} with TeeKeyManager..."
    # Encode the address as publicValues (MockAttestationVerifier just decodes and returns it)
    PUBLIC_VALUES=$(cast abi-encode "f(address)" "{{ address }}")
    cast send --rpc-url {{ anvil_rpc_url }} \
        --private-key {{ anvil_key_0 }} \
        {{ tee_key_manager_contract_address }} \
        "addKey(bytes,bytes)" \
        "$PUBLIC_VALUES" \
        "0x"
    echo "Done. Key {{ address }} is now registered."

# Register the sequencer's dynamically generated key as a TEE key

# Usage: just register-sequencer-key
[group('components')]
register-sequencer-key:
    #!/usr/bin/env bash
    set -euo pipefail
    echo "Fetching sequencer address..."
    SIGNER_ADDRESS=$(curl -s "http://127.0.0.1:{{ sequencer_port }}/status" | jq -r '.signer_address')
    if [ -z "$SIGNER_ADDRESS" ] || [ "$SIGNER_ADDRESS" = "null" ]; then
        echo "Error: Could not fetch signer_address from sequencer"
        exit 1
    fi
    echo "Registering sequencer key $SIGNER_ADDRESS with TeeKeyManager..."
    PUBLIC_VALUES=$(cast abi-encode "f(address)" "$SIGNER_ADDRESS")
    cast send --rpc-url {{ anvil_rpc_url }} \
        --private-key {{ anvil_key_0 }} \
        {{ tee_key_manager_contract_address }} \
        "addKey(bytes,bytes)" \
        "$PUBLIC_VALUES" \
        "0x"
    echo "Done. Sequencer key $SIGNER_ADDRESS is now registered as TEE key."

# Register the validator's bridge signer key as a TEE key

# Usage: just register-validator-key
[group('components')]
register-validator-key:
    #!/usr/bin/env bash
    set -euo pipefail
    echo "Fetching validator bridge signer address..."
    SIGNER_ADDRESS=$(curl -s "http://127.0.0.1:8081/info" | jq -r '.signer')
    if [ -z "$SIGNER_ADDRESS" ] || [ "$SIGNER_ADDRESS" = "null" ]; then
        echo "Error: Could not fetch signer from validator (is bridge signer mode enabled?)"
        exit 1
    fi
    echo "Registering validator key $SIGNER_ADDRESS with TeeKeyManager..."
    PUBLIC_VALUES=$(cast abi-encode "f(address)" "$SIGNER_ADDRESS")
    cast send --rpc-url {{ anvil_rpc_url }} \
        --private-key {{ anvil_key_0 }} \
        {{ tee_key_manager_contract_address }} \
        "addKey(bytes,bytes)" \
        "$PUBLIC_VALUES" \
        "0x"
    echo "Done. Validator key $SIGNER_ADDRESS is now registered as TEE key."

# Run sequencer (key is generated at startup)
[group('components')]
sequencer:
    mkdir -p {{ data_dir }}
    BIND_ADDRESS=127.0.0.1:{{ sequencer_port }} \
    DATABASE_PATH={{ data_dir }}/sequencer.db \
    cargo run -p synddb-sequencer --release

# Run validator with local defaults (fetches pubkey from sequencer)
[group('components')]
validator:
    mkdir -p {{ data_dir }}
    SEQUENCER_URL=http://127.0.0.1:{{ sequencer_port }} \
    DATABASE_PATH={{ data_dir }}/validator.db \
    STATE_DB_PATH={{ data_dir }}/validator_state.db \
    PENDING_CHANGESETS_DB_PATH={{ data_dir }}/pending_changesets.db \
    cargo run -p synddb-validator --release

# Run validator with bridge signer enabled (key generated at startup, fetches pubkey from sequencer)

# Note: If on-chain operations are needed, run `just fund-sequencer` after startup
[group('components')]
validator-bridge:
    mkdir -p {{ data_dir }}
    SEQUENCER_URL=http://127.0.0.1:{{ sequencer_port }} \
    DATABASE_PATH={{ data_dir }}/validator.db \
    STATE_DB_PATH={{ data_dir }}/validator_state.db \
    PENDING_CHANGESETS_DB_PATH={{ data_dir }}/pending_changesets.db \
    BRIDGE_SIGNER=true \
    BRIDGE_CONTRACT_ADDRESS={{ bridge_contract_address }} \
    BRIDGE_CHAIN_ID=31337 \
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

# ============================================================================
# Reproducible Builds (for Confidential Space / TEE verification)
# ============================================================================
# BuildKit version pinned for reproducibility (shared with CI via docker/reproducible/buildkit.version)

buildkit_version := "moby/buildkit:" + trim(read("docker/reproducible/buildkit.version"))
buildkit_builder := "synddb-repro"

# Set up BuildKit builder with pinned version for reproducible builds
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

# Build reproducible sequencer image (distroless)
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

# Build reproducible validator image (distroless)
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

# Build debug sequencer image (has shell)
[group('reproducible')]
repro-sequencer-debug: repro-setup
    SOURCE_DATE_EPOCH=0 docker buildx build \
        --builder {{ buildkit_builder }} \
        --no-cache \
        --provenance=false \
        --sbom=false \
        --build-arg SOURCE_DATE_EPOCH=0 \
        --platform linux/amd64 \
        --output type=docker,rewrite-timestamp=true \
        --target debug \
        -f docker/reproducible/sequencer.Dockerfile \
        -t synddb-sequencer:debug .

# Build debug validator image (has shell)
[group('reproducible')]
repro-validator-debug: repro-setup
    SOURCE_DATE_EPOCH=0 docker buildx build \
        --builder {{ buildkit_builder }} \
        --no-cache \
        --provenance=false \
        --sbom=false \
        --build-arg SOURCE_DATE_EPOCH=0 \
        --platform linux/amd64 \
        --output type=docker,rewrite-timestamp=true \
        --target debug \
        -f docker/reproducible/validator.Dockerfile \
        -t synddb-validator:debug .

# Build all reproducible images (production)
[group('reproducible')]
repro-all: repro-sequencer repro-validator

# Build all debug images
[group('reproducible')]
repro-all-debug: repro-sequencer-debug repro-validator-debug

# Show image digests for verification
[group('reproducible')]
repro-verify:
    #!/usr/bin/env bash
    set -euo pipefail
    echo "=== Reproducible Image Digests ==="
    echo ""
    echo "Sequencer:"
    docker inspect synddb-sequencer:reproducible --format='  Image ID: {{ "{{" }}.Id{{ "}}" }}' 2>/dev/null || echo "  (not built)"
    docker inspect synddb-sequencer:reproducible --format='  Created:  {{ "{{" }}.Created{{ "}}" }}' 2>/dev/null || true
    echo ""
    echo "Validator:"
    docker inspect synddb-validator:reproducible --format='  Image ID: {{ "{{" }}.Id{{ "}}" }}' 2>/dev/null || echo "  (not built)"
    docker inspect synddb-validator:reproducible --format='  Created:  {{ "{{" }}.Created{{ "}}" }}' 2>/dev/null || true

# Artifact Registry configuration

ar_registry := "us-central1-docker.pkg.dev/synddb-infra/synddb"

# Verify local builds match published images
[group('reproducible')]
verify-build tag="edge":
    #!/usr/bin/env bash
    set -euo pipefail

    echo "=== Building Reproducible Images Locally ==="
    echo ""
    just repro-all

    echo ""
    echo "=== Fetching Published Images ==="
    echo ""
    docker pull {{ ar_registry }}/synddb-sequencer:{{ tag }}
    docker pull {{ ar_registry }}/synddb-validator:{{ tag }}

    echo ""
    echo "=== Comparing Image Hashes ==="
    echo ""

    LOCAL_SEQ=$(docker inspect synddb-sequencer:reproducible --format='{{ "{{" }}.Id{{ "}}" }}')
    LOCAL_VAL=$(docker inspect synddb-validator:reproducible --format='{{ "{{" }}.Id{{ "}}" }}')
    REMOTE_SEQ=$(docker inspect {{ ar_registry }}/synddb-sequencer:{{ tag }} --format='{{ "{{" }}.Id{{ "}}" }}')
    REMOTE_VAL=$(docker inspect {{ ar_registry }}/synddb-validator:{{ tag }} --format='{{ "{{" }}.Id{{ "}}" }}')

    echo "Sequencer:"
    echo "  Local:  $LOCAL_SEQ"
    echo "  Remote: $REMOTE_SEQ"
    if [ "$LOCAL_SEQ" = "$REMOTE_SEQ" ]; then
        echo "  ✓ MATCH"
    else
        echo "  ✗ MISMATCH"
        FAILED=1
    fi

    echo ""
    echo "Validator:"
    echo "  Local:  $LOCAL_VAL"
    echo "  Remote: $REMOTE_VAL"
    if [ "$LOCAL_VAL" = "$REMOTE_VAL" ]; then
        echo "  ✓ MATCH"
    else
        echo "  ✗ MISMATCH"
        FAILED=1
    fi

    echo ""
    if [ "${FAILED:-0}" = "1" ]; then
        echo "=== VERIFICATION FAILED ==="
        exit 1
    else
        echo "=== All Images Verified! ==="
    fi

# Verify cosign signatures on published images
[group('reproducible')]
verify-signatures tag="edge":
    #!/usr/bin/env bash
    set -euo pipefail

    echo "=== Verifying Cosign Signatures ==="
    echo ""
    echo "Sequencer:"
    cosign verify {{ ar_registry }}/synddb-sequencer:{{ tag }} \
        --certificate-identity-regexp='https://github.com/SyndicateProtocol/synddb/.*' \
        --certificate-oidc-issuer='https://token.actions.githubusercontent.com' \
        | head -20

    echo ""
    echo "Validator:"
    cosign verify {{ ar_registry }}/synddb-validator:{{ tag }} \
        --certificate-identity-regexp='https://github.com/SyndicateProtocol/synddb/.*' \
        --certificate-oidc-issuer='https://token.actions.githubusercontent.com' \
        | head -20

    echo ""
    echo "=== All Signatures Verified! ==="

# Base images for reproducible builds (update version tags here when upgrading)

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
    [ "$RUST_LATEST" = "$CURRENT" ] && echo "  ✓ Up to date" || echo "  ⚠ Update available"
    echo ""

    echo "{{ repro_debian_image }}"
    echo "  Latest: $DEBIAN_LATEST"
    CURRENT=$(grep 'ARG DEBIAN_IMAGE_DIGEST=' docker/reproducible/sequencer.Dockerfile | head -1 | cut -d= -f2)
    echo "  Current: $CURRENT"
    [ "$DEBIAN_LATEST" = "$CURRENT" ] && echo "  ✓ Up to date" || echo "  ⚠ Update available"
    echo ""

    echo "{{ repro_distroless_image }}"
    echo "  Latest: $DISTROLESS_LATEST"
    CURRENT=$(grep 'ARG DISTROLESS_IMAGE_DIGEST=' docker/reproducible/sequencer.Dockerfile | head -1 | cut -d= -f2)
    echo "  Current: $CURRENT"
    [ "$DISTROLESS_LATEST" = "$CURRENT" ] && echo "  ✓ Up to date" || echo "  ⚠ Update available"

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
# Terraform (Infrastructure)
# ============================================================================

# Initialize Terraform for an environment
[group('terraform')]
tf-init env="staging":
    cd deploy/terraform/environments/{{ env }} && terraform init

# Plan Terraform changes for an environment
[group('terraform')]
tf-plan env="staging":
    cd deploy/terraform/environments/{{ env }} && terraform plan

# Apply Terraform changes for an environment
[group('terraform')]
tf-apply env="staging":
    cd deploy/terraform/environments/{{ env }} && terraform apply

# Destroy Terraform resources for an environment
[confirm('This will DESTROY all resources in {{ env }}. Are you sure?')]
[group('terraform')]
tf-destroy env="staging":
    cd deploy/terraform/environments/{{ env }} && terraform destroy

# Show Terraform outputs for an environment
[group('terraform')]
tf-output env="staging":
    cd deploy/terraform/environments/{{ env }} && terraform output

# Format all Terraform files
[group('terraform')]
tf-fmt:
    terraform fmt -recursive deploy/terraform/

# Validate Terraform configuration
[group('terraform')]
tf-validate env="staging":
    cd deploy/terraform/environments/{{ env }} && terraform validate

# Initialize and plan Marketplace package
[group('terraform')]
tf-marketplace-plan:
    cd deploy/terraform/marketplace && terraform init && terraform plan -var="goog_cm_deployment_name=test"
