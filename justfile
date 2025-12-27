# SyndDB Development Commands
#
# Install just: https://github.com/casey/just
#   brew install just
#   cargo install just
#
# Usage:
#   just                    # Show available commands
#   just dev                # Start full local environment
#   just deploy             # Deploy contracts to Anvil
#   just sequencer          # Run sequencer only
#   just validator          # Run validator only
#   just test               # Run all tests
#   just check              # Run all CI checks
#
# Modules:
#   just contracts::build   # Build Solidity contracts
#   just contracts::test    # Run contract tests
#   just examples::price-oracle       # Run price oracle example
#   just examples::prediction-market  # Run prediction market example

# ============================================================================
# Modules
# ============================================================================

mod contracts 'contracts/mod.just'
mod examples 'examples/mod.just'

# ============================================================================
# Shared Configuration (Single Source of Truth)
# ============================================================================

# Well-known Anvil addresses and keys (DO NOT USE IN PRODUCTION)
anvil_key_0 := "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
anvil_address_0 := "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"
sequencer_pubkey := "8318535b54105d4a7aae60c08fc45f9687181b4fdfc625bd1a753fa7397fed753547f11ca8696646f2f3acb08e31016afac23e630c5d11f59f61fef57b0d2aa5"

# Deterministic contract addresses (from deploy-local.sh with Anvil account 0)
bridge_address := "0xe7f1725E7734CE288F8367e1Bb143E90bb3F0512"
weth_address := "0x5FbDB2315678afecb367f032d93F642f64180aa3"
price_oracle_address := "0xCf7Ed3AccA5a467e9e704C703E8D87F634fB0Fc9"

# Default ports
anvil_port := "8545"
sequencer_port := "8433"
validator_port := "8080"

# Data directories
data_dir := "./data"

# ============================================================================
# Default & Help
# ============================================================================

# Show available commands
default:
    @just --list

# ============================================================================
# Local Development
# ============================================================================

# Start full local dev environment (Anvil + contracts + sequencer)
dev:
    ./scripts/dev-env.sh

# Start dev environment with validator
dev-validator:
    ./scripts/dev-env.sh --validator

# Deploy contracts to local Anvil (starts Anvil if not running)
deploy:
    ./scripts/deploy-local.sh

# Deploy with fresh Anvil (kills existing)
deploy-fresh:
    ./scripts/deploy-local.sh --reset

# ============================================================================
# Components
# ============================================================================

# Start Anvil only
anvil:
    anvil --port {{anvil_port}}

# Run sequencer with local defaults
sequencer:
    mkdir -p {{data_dir}}
    SIGNING_KEY={{anvil_key_0}} \
    BIND_ADDRESS=127.0.0.1:{{sequencer_port}} \
    DATABASE_PATH={{data_dir}}/sequencer.db \
    cargo run -p synddb-sequencer --release

# Run sequencer (release binary, faster startup)
sequencer-release:
    mkdir -p {{data_dir}}
    SIGNING_KEY={{anvil_key_0}} \
    BIND_ADDRESS=127.0.0.1:{{sequencer_port}} \
    DATABASE_PATH={{data_dir}}/sequencer.db \
    ./target/release/synddb-sequencer

# Run validator with local defaults
validator:
    mkdir -p {{data_dir}}
    SEQUENCER_PUBKEY={{sequencer_pubkey}} \
    SEQUENCER_URL=http://127.0.0.1:{{sequencer_port}} \
    DATABASE_PATH={{data_dir}}/validator.db \
    STATE_DB_PATH={{data_dir}}/validator_state.db \
    PENDING_CHANGESETS_DB_PATH={{data_dir}}/pending_changesets.db \
    cargo run -p synddb-validator --release

# Run validator (release binary, faster startup)
validator-release:
    mkdir -p {{data_dir}}
    SEQUENCER_PUBKEY={{sequencer_pubkey}} \
    SEQUENCER_URL=http://127.0.0.1:{{sequencer_port}} \
    DATABASE_PATH={{data_dir}}/validator.db \
    STATE_DB_PATH={{data_dir}}/validator_state.db \
    PENDING_CHANGESETS_DB_PATH={{data_dir}}/pending_changesets.db \
    ./target/release/synddb-validator

# Run validator with bridge signer enabled
validator-bridge:
    mkdir -p {{data_dir}}
    SEQUENCER_PUBKEY={{sequencer_pubkey}} \
    SEQUENCER_URL=http://127.0.0.1:{{sequencer_port}} \
    DATABASE_PATH={{data_dir}}/validator.db \
    STATE_DB_PATH={{data_dir}}/validator_state.db \
    PENDING_CHANGESETS_DB_PATH={{data_dir}}/pending_changesets.db \
    BRIDGE_SIGNER=true \
    BRIDGE_CONTRACT={{bridge_address}} \
    BRIDGE_CHAIN_ID=31337 \
    BRIDGE_SIGNING_KEY={{anvil_key_0}} \
    ./target/release/synddb-validator

# ============================================================================
# Building
# ============================================================================

# Build all crates (debug)
build:
    cargo build --workspace

# Build all crates (release)
build-release:
    cargo build --workspace --release

# ============================================================================
# Testing
# ============================================================================

# Run all tests
test:
    cargo nextest run --workspace --all-features --exclude synddb-e2e --exclude synddb-e2e-gcs

# Run tests with output
test-verbose:
    cargo nextest run --workspace --all-features --exclude synddb-e2e --exclude synddb-e2e-gcs --no-capture

# Run fuzzer
fuzz:
    cargo test -p synddb-fuzzer --release

# Run E2E fuzzer
fuzz-e2e:
    cargo test -p synddb-e2e-fuzzer --release

# ============================================================================
# Code Quality
# ============================================================================

# Run all CI checks (non-destructive)
check:
    @echo "Checking TOML formatting..."
    taplo lint "**/Cargo.toml"
    taplo fmt --check "**/Cargo.toml"
    @echo "Checking for unused dependencies..."
    cargo machete
    @echo "Checking Rust formatting..."
    cargo +nightly fmt --all --check
    @echo "Running Clippy..."
    cargo clippy --workspace --all-targets --all-features
    @echo "All checks passed!"

# Fix all auto-fixable issues
fix:
    taplo fmt "**/Cargo.toml"
    cargo +nightly fmt --all
    cargo clippy --workspace --all-targets --all-features --fix --allow-dirty --allow-staged

# Format all code (Rust, TOML, Solidity)
fmt:
    cargo +nightly fmt --all
    taplo fmt "**/Cargo.toml"
    cd contracts && forge fmt

# ============================================================================
# Cleanup
# ============================================================================

# Clean build artifacts
clean:
    cargo clean
    cd contracts && forge clean

# Clean local development data
clean-data:
    rm -rf ./data
    rm -rf ./.synddb

# Clean everything
clean-all: clean clean-data examples::clean

# Kill any running Anvil processes
kill-anvil:
    pkill -f "anvil" || true

# ============================================================================
# Documentation
# ============================================================================

# Build and open docs
docs:
    cargo doc --workspace --all-features --no-deps --open

# ============================================================================
# Docker
# ============================================================================

# Run with Docker Compose
docker-up:
    docker compose up --build

# Run with Docker Compose (detached)
docker-up-detached:
    docker compose up --build -d

# Stop Docker Compose
docker-down:
    docker compose down -v
