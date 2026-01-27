# SyndDB Examples Module
#
# Usage:
#   just examples::price-oracle        # Run price oracle example
#   just examples::prediction-market   # Run prediction market CLI
#   just examples::prediction-market-http  # Run prediction market HTTP server
#   just examples::clean               # Clean example data
#
# All recipes use [no-cd] to run from the project root since they
# depend on binaries and configuration from the root directory.
#
# Note: Sequencer keys are generated dynamically at startup (TEE security model).
# If you need to fund the sequencer's wallet for on-chain operations, use:
#   just fund-sequencer

# Use bash with strict mode
set shell := ["bash", "-euo", "pipefail", "-c"]

# Shared configuration
sequencer_port := "8433"
validator_port := "8080"
data_dir := "./data"

# --- Price Oracle Example ---

# Build price oracle validator
[no-cd]
[group('price-oracle')]
[private]
_build-price-oracle:
    cargo build --release -p price-oracle-validator

# Setup Python environment for price oracle
[group('price-oracle')]
[private]
_setup-price-oracle:
    cd price-oracle && python3 -m venv venv && ./venv/bin/pip install -q -r app/requirements.txt

# Deploy contracts (calls parent recipe)
[no-cd]
[group('price-oracle')]
[private]
_deploy-contracts:
    just deploy

# Run price oracle example (full environment)
[no-cd]
[group('price-oracle')]
price-oracle: _build-price-oracle _setup-price-oracle _deploy-contracts
    #!/usr/bin/env bash
    set -euxo pipefail
    trap 'kill $(jobs -p) 2>/dev/null' EXIT

    echo "Starting sequencer..."
    mkdir -p {{data_dir}}/price-oracle
    BIND_ADDRESS=127.0.0.1:{{sequencer_port}} \
    DATABASE_PATH={{data_dir}}/price-oracle/sequencer.db \
    ./target/release/synddb-sequencer &
    sleep 2

    echo "Starting price oracle validator..."
    SEQUENCER_URL=http://127.0.0.1:{{sequencer_port}} \
    DATABASE_PATH={{data_dir}}/price-oracle/validator.db \
    STATE_DB_PATH={{data_dir}}/price-oracle/validator_state.db \
    PENDING_CHANGESETS_DB_PATH={{data_dir}}/price-oracle/pending_changesets.db \
    BIND_ADDRESS=127.0.0.1:{{validator_port}} \
    MAX_PRICE_DIFFERENCE_BPS=100 \
    ./target/release/price-oracle-validator &
    sleep 2

    echo "Initializing price oracle database..."
    cd examples/price-oracle
    ./venv/bin/python -m app.main --db ../../{{data_dir}}/price-oracle/prices.db init

    echo "Starting price fetcher..."
    ./venv/bin/python -m app.fetcher \
        --db ../../{{data_dir}}/price-oracle/prices.db \
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
[no-cd]
[group('prediction-market')]
[private]
_build-prediction-market:
    cargo build --release -p prediction-market --features chain-monitor

# Run prediction market example (CLI mode)
[no-cd]
[group('prediction-market')]
prediction-market: _build-prediction-market _deploy-contracts
    #!/usr/bin/env bash
    set -euxo pipefail
    trap 'kill $(jobs -p) 2>/dev/null' EXIT

    echo "Starting sequencer..."
    mkdir -p {{data_dir}}/prediction-market
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
[no-cd]
[group('prediction-market')]
prediction-market-http: _build-prediction-market _deploy-contracts
    #!/usr/bin/env bash
    set -euxo pipefail
    trap 'kill $(jobs -p) 2>/dev/null' EXIT

    echo "Starting sequencer..."
    mkdir -p {{data_dir}}/prediction-market
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

# --- Cleanup ---

# Clean example data
[no-cd]
[group('cleanup')]
[confirm('This will remove all example data. Continue?')]
clean:
    rm -rf {{data_dir}}/price-oracle
    rm -rf {{data_dir}}/prediction-market
    rm -rf examples/price-oracle/.dev-data
    rm -rf examples/prediction-market/.dev-data
