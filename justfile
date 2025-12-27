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
# Examples:
#   just example-price-oracle       # Run price oracle example
#   just example-prediction-market  # Run prediction market example

# Default recipe shows help
default:
    @just --list

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
# Local Development
# ============================================================================

# Start full local dev environment (Anvil + contracts + sequencer + validator)
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
# Individual Components
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

# Build contracts
build-contracts:
    cd contracts && forge build

# ============================================================================
# Testing
# ============================================================================

# Run all tests
test:
    cargo nextest run --workspace --all-features --exclude synddb-e2e --exclude synddb-e2e-gcs

# Run tests with output
test-verbose:
    cargo nextest run --workspace --all-features --exclude synddb-e2e --exclude synddb-e2e-gcs --no-capture

# Run contract tests
test-contracts:
    cd contracts && forge test -vvv

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

# Format code
fmt:
    cargo +nightly fmt --all
    taplo fmt "**/Cargo.toml"
    cd contracts && forge fmt

# Lint contracts
lint-contracts:
    cd contracts && forge fmt --check

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
clean-all: clean clean-data

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

# ============================================================================
# Examples
# ============================================================================

# --- Price Oracle Example ---

# Build price oracle validator
build-price-oracle:
    cargo build --release -p price-oracle-validator

# Setup Python environment for price oracle
setup-price-oracle:
    cd examples/price-oracle && python3 -m venv venv && ./venv/bin/pip install -q -r app/requirements.txt

# Run price oracle example (full environment)
example-price-oracle: build-release build-price-oracle setup-price-oracle deploy
    #!/usr/bin/env bash
    set -e
    trap 'kill $(jobs -p) 2>/dev/null' EXIT

    echo "Starting sequencer..."
    SIGNING_KEY={{anvil_key_0}} \
    BIND_ADDRESS=127.0.0.1:{{sequencer_port}} \
    DATABASE_PATH={{data_dir}}/price-oracle/sequencer.db \
    ./target/release/synddb-sequencer &
    sleep 2

    echo "Starting price oracle validator..."
    mkdir -p {{data_dir}}/price-oracle
    SEQUENCER_PUBKEY={{sequencer_pubkey}} \
    SEQUENCER_URL=http://127.0.0.1:{{sequencer_port}} \
    DATABASE_PATH={{data_dir}}/price-oracle/validator.db \
    STATE_DB_PATH={{data_dir}}/price-oracle/validator_state.db \
    BIND_ADDRESS=127.0.0.1:{{validator_port}} \
    MAX_PRICE_DIFFERENCE_BPS=100 \
    ./target/release/price-oracle-validator &
    sleep 2

    echo "Initializing price oracle database..."
    cd examples/price-oracle
    ./venv/bin/python -m app.main --db {{data_dir}}/price-oracle/prices.db init

    echo "Starting price fetcher..."
    ./venv/bin/python -m app.fetcher \
        --db {{data_dir}}/price-oracle/prices.db \
        --sequencer-url http://127.0.0.1:{{sequencer_port}} \
        --interval 10 \
        --mock \
        -v &

    echo ""
    echo "Price Oracle Example Running"
    echo "  Sequencer:  http://127.0.0.1:{{sequencer_port}}"
    echo "  Validator:  http://127.0.0.1:{{validator_port}}"
    echo "  Press Ctrl+C to stop"
    echo ""
    wait

# --- Prediction Market Example ---

# Build prediction market
build-prediction-market:
    cargo build --release -p prediction-market --features chain-monitor

# Run prediction market example (full environment)
example-prediction-market: build-release build-prediction-market deploy
    #!/usr/bin/env bash
    set -e
    trap 'kill $(jobs -p) 2>/dev/null' EXIT

    echo "Starting sequencer..."
    mkdir -p {{data_dir}}/prediction-market
    SIGNING_KEY={{anvil_key_0}} \
    BIND_ADDRESS=127.0.0.1:{{sequencer_port}} \
    DATABASE_PATH={{data_dir}}/prediction-market/sequencer.db \
    ./target/release/synddb-sequencer &
    sleep 2

    echo "Initializing prediction market..."
    PM="./target/release/prediction-market"
    PM_DB="{{data_dir}}/prediction-market/market.db"
    $PM --db "$PM_DB" --sequencer "http://127.0.0.1:{{sequencer_port}}" init

    echo "Creating demo accounts and market..."
    $PM --db "$PM_DB" --sequencer "http://127.0.0.1:{{sequencer_port}}" create-account alice
    $PM --db "$PM_DB" --sequencer "http://127.0.0.1:{{sequencer_port}}" create-account bob
    $PM --db "$PM_DB" --sequencer "http://127.0.0.1:{{sequencer_port}}" create-market "Will ETH hit 5k in 2025?" --resolution-time 1767225600

    sleep 2

    echo "Starting validator..."
    SEQUENCER_PUBKEY={{sequencer_pubkey}} \
    SEQUENCER_URL=http://127.0.0.1:{{sequencer_port}} \
    DATABASE_PATH={{data_dir}}/prediction-market/validator.db \
    STATE_DB_PATH={{data_dir}}/prediction-market/validator_state.db \
    BIND_ADDRESS=127.0.0.1:{{validator_port}} \
    SYNC_INTERVAL=1s \
    ./target/release/synddb-validator &
    sleep 2

    echo ""
    echo "Prediction Market Example Running"
    echo "  Sequencer:  http://127.0.0.1:{{sequencer_port}}"
    echo "  Validator:  http://127.0.0.1:{{validator_port}}"
    echo ""
    echo "Run commands with:"
    echo "  $PM --db $PM_DB --sequencer http://127.0.0.1:{{sequencer_port}} status"
    echo "  $PM --db $PM_DB --sequencer http://127.0.0.1:{{sequencer_port}} buy --account 1 --market 1 --outcome yes --shares 50"
    echo ""
    echo "Press Ctrl+C to stop"
    wait

# Run prediction market HTTP server
example-prediction-market-http: build-release build-prediction-market deploy
    #!/usr/bin/env bash
    set -e
    trap 'kill $(jobs -p) 2>/dev/null' EXIT

    echo "Starting sequencer..."
    mkdir -p {{data_dir}}/prediction-market
    SIGNING_KEY={{anvil_key_0}} \
    BIND_ADDRESS=127.0.0.1:{{sequencer_port}} \
    DATABASE_PATH={{data_dir}}/prediction-market/sequencer.db \
    ./target/release/synddb-sequencer &
    sleep 2

    echo "Initializing prediction market..."
    PM="./target/release/prediction-market"
    PM_DB="{{data_dir}}/prediction-market/market.db"
    $PM --db "$PM_DB" --sequencer "http://127.0.0.1:{{sequencer_port}}" init

    echo "Starting HTTP server on port 8081..."
    $PM --db "$PM_DB" --sequencer "http://127.0.0.1:{{sequencer_port}}" serve --port 8081 &
    sleep 2

    echo "Starting validator..."
    SEQUENCER_PUBKEY={{sequencer_pubkey}} \
    SEQUENCER_URL=http://127.0.0.1:{{sequencer_port}} \
    DATABASE_PATH={{data_dir}}/prediction-market/validator.db \
    STATE_DB_PATH={{data_dir}}/prediction-market/validator_state.db \
    BIND_ADDRESS=127.0.0.1:{{validator_port}} \
    SYNC_INTERVAL=1s \
    ./target/release/synddb-validator &
    sleep 2

    echo ""
    echo "Prediction Market HTTP Server Running"
    echo "  API:        http://127.0.0.1:8081"
    echo "  Sequencer:  http://127.0.0.1:{{sequencer_port}}"
    echo "  Validator:  http://127.0.0.1:{{validator_port}}"
    echo ""
    echo "API Examples:"
    echo "  curl http://127.0.0.1:8081/status | jq ."
    echo "  curl -X POST http://127.0.0.1:8081/accounts -H 'Content-Type: application/json' -d '{\"name\": \"charlie\"}'"
    echo ""
    echo "Press Ctrl+C to stop"
    wait

# Clean example data
clean-examples:
    rm -rf {{data_dir}}/price-oracle
    rm -rf {{data_dir}}/prediction-market
    rm -rf examples/price-oracle/.dev-data
    rm -rf examples/prediction-market/.dev-data
