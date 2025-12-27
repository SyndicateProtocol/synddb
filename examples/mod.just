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

# Shared configuration (mirrors root justfile)
anvil_key_0 := "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
sequencer_pubkey := "8318535b54105d4a7aae60c08fc45f9687181b4fdfc625bd1a753fa7397fed753547f11ca8696646f2f3acb08e31016afac23e630c5d11f59f61fef57b0d2aa5"
sequencer_port := "8433"
validator_port := "8080"
data_dir := "./data"

# --- Price Oracle Example ---

# Build price oracle validator
[no-cd]
build-price-oracle:
    cargo build --release -p price-oracle-validator

# Setup Python environment for price oracle
setup-price-oracle:
    cd price-oracle && python3 -m venv venv && ./venv/bin/pip install -q -r app/requirements.txt

# Run price oracle example (full environment)
[no-cd]
price-oracle: build-price-oracle setup-price-oracle
    just deploy
    #!/usr/bin/env bash
    set -e
    trap 'kill $(jobs -p) 2>/dev/null' EXIT

    echo "Starting sequencer..."
    mkdir -p {{data_dir}}/price-oracle
    SIGNING_KEY={{anvil_key_0}} \
    BIND_ADDRESS=127.0.0.1:{{sequencer_port}} \
    DATABASE_PATH={{data_dir}}/price-oracle/sequencer.db \
    ./target/release/synddb-sequencer &
    sleep 2

    echo "Starting price oracle validator..."
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
build-prediction-market:
    cargo build --release -p prediction-market --features chain-monitor

# Run prediction market example (CLI mode)
[no-cd]
prediction-market: build-prediction-market
    just deploy
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
[no-cd]
prediction-market-http: build-prediction-market
    just deploy
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

# --- Cleanup ---

# Clean example data
[no-cd]
clean:
    rm -rf {{data_dir}}/price-oracle
    rm -rf {{data_dir}}/prediction-market
    rm -rf examples/price-oracle/.dev-data
    rm -rf examples/prediction-market/.dev-data
