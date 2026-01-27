#!/usr/bin/env bash
#
# SyndDB Development Environment
#
# This script orchestrates a complete local development environment demonstrating
# the full SyndDB architecture:
#
#   1. Anvil (local Ethereum node)
#   2. TestBridge contract deployment
#   3. Sequencer (with local SQLite storage)
#   4. Prediction Market application
#   5. Validator (reconstructs state from sequencer)
#   6. Chain Monitor (optional - watches bridge events)
#
# Usage:
#   ./scripts/dev-env.sh              # Run full demo with CLI
#   ./scripts/dev-env.sh --http       # Run demo with HTTP server
#   ./scripts/dev-env.sh --no-monitor # Skip chain monitor
#   ./scripts/dev-env.sh --cleanup    # Clean up data files
#
# Requirements:
#   - Rust toolchain (cargo)
#   - Foundry (anvil, forge, cast)
#

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"
EXAMPLE_DIR="$PROJECT_ROOT/examples/prediction-market"
CONTRACTS_DIR="$PROJECT_ROOT/contracts"
DATA_DIR="$EXAMPLE_DIR/.dev-data"

# Ports
ANVIL_PORT=8545
SEQUENCER_PORT=8433
VALIDATOR_PORT=8434
APP_PORT=8080

# Test private key (anvil default account 0)
# WARNING: Never use this key with real funds!
PRIVATE_KEY="0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
# Corresponding public key (64 bytes, uncompressed, no 04 prefix)
SEQUENCER_PUBKEY="8318535b54105d4a7aae60c08fc45f9687181b4fdfc625bd1a753fa7397fed753547f11ca8696646f2f3acb08e31016afac23e630c5d11f59f61fef57b0d2aa5"

# Process tracking
ANVIL_PID=""
SEQUENCER_PID=""
VALIDATOR_PID=""
APP_PID=""

cleanup() {
    echo -e "\n${YELLOW}Cleaning up...${NC}"

    if [ -n "$APP_PID" ] && kill -0 "$APP_PID" 2>/dev/null; then
        echo "Stopping app (PID $APP_PID)"
        kill "$APP_PID" 2>/dev/null || true
    fi

    if [ -n "$VALIDATOR_PID" ] && kill -0 "$VALIDATOR_PID" 2>/dev/null; then
        echo "Stopping validator (PID $VALIDATOR_PID)"
        kill "$VALIDATOR_PID" 2>/dev/null || true
    fi

    if [ -n "$SEQUENCER_PID" ] && kill -0 "$SEQUENCER_PID" 2>/dev/null; then
        echo "Stopping sequencer (PID $SEQUENCER_PID)"
        kill "$SEQUENCER_PID" 2>/dev/null || true
    fi

    if [ -n "$ANVIL_PID" ] && kill -0 "$ANVIL_PID" 2>/dev/null; then
        echo "Stopping anvil (PID $ANVIL_PID)"
        kill "$ANVIL_PID" 2>/dev/null || true
    fi

    # Wait for processes to terminate
    sleep 1

    echo -e "${GREEN}Cleanup complete${NC}"
}

trap cleanup EXIT

print_header() {
    echo -e "\n${BLUE}════════════════════════════════════════════════════════════════${NC}"
    echo -e "${BLUE}  $1${NC}"
    echo -e "${BLUE}════════════════════════════════════════════════════════════════${NC}\n"
}

print_step() {
    echo -e "${GREEN}▶ $1${NC}"
}

print_info() {
    echo -e "${YELLOW}  ℹ $1${NC}"
}

wait_for_port() {
    local port=$1
    local name=$2
    local max_attempts=30
    local attempt=0

    echo -n "  Waiting for $name on port $port"
    while ! nc -z localhost "$port" 2>/dev/null; do
        attempt=$((attempt + 1))
        if [ $attempt -ge $max_attempts ]; then
            echo -e " ${RED}TIMEOUT${NC}"
            return 1
        fi
        echo -n "."
        sleep 0.5
    done
    echo -e " ${GREEN}OK${NC}"
}

# Parse arguments
SKIP_MONITOR=false
CLEANUP_ONLY=false
HTTP_MODE=false

for arg in "$@"; do
    case $arg in
        --no-monitor)
            SKIP_MONITOR=true
            ;;
        --cleanup)
            CLEANUP_ONLY=true
            ;;
        --http)
            HTTP_MODE=true
            ;;
    esac
done

if [ "$CLEANUP_ONLY" = true ]; then
    print_header "Cleaning up data files"
    rm -rf "$DATA_DIR"
    echo "Removed $DATA_DIR"
    exit 0
fi

print_header "SyndDB Development Environment"

echo "Project root: $PROJECT_ROOT"
echo "Data directory: $DATA_DIR"
echo ""

# Create data directory
mkdir -p "$DATA_DIR"

# ============================================================================
# Step 1: Build everything
# ============================================================================
print_header "Step 1: Building Components"

print_step "Building Rust crates..."
cd "$PROJECT_ROOT"
cargo build --release -p synddb-sequencer -p synddb-validator -p prediction-market --features chain-monitor 2>&1 | tail -5

print_step "Building contracts..."
cd "$CONTRACTS_DIR"
forge build --quiet 2>&1 || true

# ============================================================================
# Step 2: Start Anvil
# ============================================================================
print_header "Step 2: Starting Anvil (Local Ethereum Node)"

print_step "Starting anvil on port $ANVIL_PORT..."
anvil --port $ANVIL_PORT --silent &
ANVIL_PID=$!
wait_for_port $ANVIL_PORT "anvil"
print_info "Anvil running with PID $ANVIL_PID"

# ============================================================================
# Step 3: Deploy TestBridge
# ============================================================================
print_header "Step 3: Deploying TestBridge Contract"

cd "$CONTRACTS_DIR"
print_step "Deploying TestBridge..."

# Deploy and capture the address
DEPLOY_OUTPUT=$(forge script script/DeployTestBridge.s.sol:DeployTestBridge \
    --rpc-url http://localhost:$ANVIL_PORT \
    --private-key $PRIVATE_KEY \
    --broadcast 2>&1)

# Extract the deployed address from the output
BRIDGE_ADDRESS=$(echo "$DEPLOY_OUTPUT" | grep -oE "TestBridge deployed to: 0x[a-fA-F0-9]{40}" | grep -oE "0x[a-fA-F0-9]{40}" | head -1)

if [ -z "$BRIDGE_ADDRESS" ]; then
    echo -e "${RED}Failed to deploy TestBridge${NC}"
    echo "$DEPLOY_OUTPUT"
    exit 1
fi

print_info "TestBridge deployed at: $BRIDGE_ADDRESS"

# ============================================================================
# Step 4: Start Sequencer
# ============================================================================
print_header "Step 4: Starting Sequencer"

print_step "Starting sequencer on port $SEQUENCER_PORT..."
SIGNING_KEY="${PRIVATE_KEY#0x}" \
LOCAL_STORAGE_PATH="$DATA_DIR/sequencer.db" \
PUBLISHER_TYPE=local \
BIND_ADDRESS="127.0.0.1:$SEQUENCER_PORT" \
"$PROJECT_ROOT/target/release/synddb-sequencer" > "$DATA_DIR/sequencer.log" 2>&1 &
SEQUENCER_PID=$!

wait_for_port $SEQUENCER_PORT "sequencer"
print_info "Sequencer running with PID $SEQUENCER_PID"
print_info "Log: $DATA_DIR/sequencer.log"

# ============================================================================
# Step 5: Run Prediction Market Demo
# ============================================================================
print_header "Step 5: Running Prediction Market Demo"

PM="$PROJECT_ROOT/target/release/prediction-market"
export SEQUENCER_URL="http://localhost:$SEQUENCER_PORT"
export PM_DATABASE="$DATA_DIR/market.db"

print_step "Initializing database..."
$PM --db "$PM_DATABASE" --sequencer "$SEQUENCER_URL" init

if [ "$HTTP_MODE" = true ]; then
    # HTTP mode: Start server and use curl for demo
    print_step "Starting HTTP server on port $APP_PORT..."
    $PM --db "$PM_DATABASE" --sequencer "$SEQUENCER_URL" serve --port $APP_PORT > "$DATA_DIR/app.log" 2>&1 &
    APP_PID=$!
    wait_for_port $APP_PORT "app"
    print_info "HTTP server running with PID $APP_PID"
    print_info "Log: $DATA_DIR/app.log"

    print_step "Creating accounts via HTTP..."
    curl -s -X POST "http://localhost:$APP_PORT/accounts" \
        -H "Content-Type: application/json" \
        -d '{"name": "alice"}' | jq .
    curl -s -X POST "http://localhost:$APP_PORT/accounts" \
        -H "Content-Type: application/json" \
        -d '{"name": "bob"}' | jq .

    print_step "Creating prediction market via HTTP..."
    curl -s -X POST "http://localhost:$APP_PORT/markets" \
        -H "Content-Type: application/json" \
        -d '{"question": "Will ETH hit 5k in 2025?", "resolution_time": 1767225600}' | jq .

    print_step "Trading via HTTP: Alice buys YES, Bob buys NO..."
    curl -s -X POST "http://localhost:$APP_PORT/markets/1/buy" \
        -H "Content-Type: application/json" \
        -d '{"account_id": 1, "outcome": "yes", "shares": 50}' | jq .
    curl -s -X POST "http://localhost:$APP_PORT/markets/1/buy" \
        -H "Content-Type: application/json" \
        -d '{"account_id": 2, "outcome": "no", "shares": 30}' | jq .

    print_step "Checking status via HTTP..."
    curl -s "http://localhost:$APP_PORT/status" | jq .
else
    # CLI mode: Use CLI commands for demo
    print_step "Creating accounts..."
    $PM --db "$PM_DATABASE" --sequencer "$SEQUENCER_URL" create-account alice
    $PM --db "$PM_DATABASE" --sequencer "$SEQUENCER_URL" create-account bob

    print_step "Creating prediction market..."
    $PM --db "$PM_DATABASE" --sequencer "$SEQUENCER_URL" create-market "Will ETH hit 5k in 2025?" --resolution-time 1767225600

    print_step "Simulating deposit from L1..."
    $PM --db "$PM_DATABASE" --sequencer "$SEQUENCER_URL" simulate-deposit \
        --tx-hash "0x$(openssl rand -hex 32)" \
        --from "0x1111111111111111111111111111111111111111" \
        --to "0xdepositor" \
        --amount 100000

    print_step "Processing deposits..."
    $PM --db "$PM_DATABASE" --sequencer "$SEQUENCER_URL" process-deposits

    print_step "Trading: Alice buys YES, Bob buys NO..."
    $PM --db "$PM_DATABASE" --sequencer "$SEQUENCER_URL" buy --account 1 --market 1 --outcome yes --shares 50
    $PM --db "$PM_DATABASE" --sequencer "$SEQUENCER_URL" buy --account 2 --market 1 --outcome no --shares 30

    print_step "Checking status..."
    $PM --db "$PM_DATABASE" --sequencer "$SEQUENCER_URL" status
fi

# Wait for changeset to be published
sleep 2

# ============================================================================
# Step 6: Start Validator
# ============================================================================
print_header "Step 6: Starting Validator (State Reconstruction)"

print_step "Starting validator on port $VALIDATOR_PORT..."
SEQUENCER_PUBKEY="$SEQUENCER_PUBKEY" \
SEQUENCER_URL="http://localhost:$SEQUENCER_PORT" \
DATABASE_PATH="$DATA_DIR/validator.db" \
STATE_DB_PATH="$DATA_DIR/validator_state.db" \
FETCHER_TYPE=http \
BIND_ADDRESS="127.0.0.1:$VALIDATOR_PORT" \
SYNC_INTERVAL=1s \
"$PROJECT_ROOT/target/release/synddb-validator" > "$DATA_DIR/validator.log" 2>&1 &
VALIDATOR_PID=$!

wait_for_port $VALIDATOR_PORT "validator"
print_info "Validator running with PID $VALIDATOR_PID"
print_info "Log: $DATA_DIR/validator.log"

# Wait for validator to sync
print_step "Waiting for validator to sync..."
sleep 3

# Check validator health
VALIDATOR_HEALTH=$(curl -s "http://localhost:$VALIDATOR_PORT/health" 2>/dev/null || echo '{"status":"unknown"}')
print_info "Validator health: $VALIDATOR_HEALTH"

# ============================================================================
# Step 7: Emit Bridge Events (for chain monitor testing)
# ============================================================================
print_header "Step 7: Emitting Bridge Events"

print_step "Simulating deposit via TestBridge contract..."
cast send "$BRIDGE_ADDRESS" "deposit(address,uint256)" \
    "0x2222222222222222222222222222222222222222" \
    "50000000000000000000" \
    --rpc-url "http://localhost:$ANVIL_PORT" \
    --private-key "$PRIVATE_KEY" \
    --quiet

print_step "Simulating withdrawal via TestBridge contract..."
cast send "$BRIDGE_ADDRESS" "withdraw(uint256,address)" \
    "25000000000000000000" \
    "0x3333333333333333333333333333333333333333" \
    --rpc-url "http://localhost:$ANVIL_PORT" \
    --private-key "$PRIVATE_KEY" \
    --quiet

print_info "Bridge events emitted. Check anvil logs or use 'cast logs' to verify."

# ============================================================================
# Summary
# ============================================================================
print_header "Development Environment Ready"

if [ "$HTTP_MODE" = true ]; then
echo -e "
${GREEN}Components running:${NC}
  - Anvil (L1):     http://localhost:$ANVIL_PORT (PID: $ANVIL_PID)
  - Sequencer:      http://localhost:$SEQUENCER_PORT (PID: $SEQUENCER_PID)
  - App (HTTP):     http://localhost:$APP_PORT (PID: $APP_PID)
  - Validator:      http://localhost:$VALIDATOR_PORT (PID: $VALIDATOR_PID)

${GREEN}Deployed contracts:${NC}
  - TestBridge:     $BRIDGE_ADDRESS

${GREEN}Data files:${NC}
  - Market DB:      $PM_DATABASE
  - Sequencer DB:   $DATA_DIR/sequencer.db
  - Validator DB:   $DATA_DIR/validator.db

${GREEN}Logs:${NC}
  - App:            $DATA_DIR/app.log
  - Sequencer:      $DATA_DIR/sequencer.log
  - Validator:      $DATA_DIR/validator.log

${YELLOW}HTTP API Examples:${NC}
  # Get system status
  curl http://localhost:$APP_PORT/status | jq .

  # List accounts
  curl http://localhost:$APP_PORT/accounts | jq .

  # Create a new account
  curl -X POST http://localhost:$APP_PORT/accounts \\
    -H 'Content-Type: application/json' \\
    -d '{\"name\": \"charlie\"}' | jq .

  # List markets
  curl http://localhost:$APP_PORT/markets | jq .

  # Buy shares
  curl -X POST http://localhost:$APP_PORT/markets/1/buy \\
    -H 'Content-Type: application/json' \\
    -d '{\"account_id\": 1, \"outcome\": \"yes\", \"shares\": 10}' | jq .

  # Resolve a market
  curl -X POST http://localhost:$APP_PORT/markets/1/resolve \\
    -H 'Content-Type: application/json' \\
    -d '{\"outcome\": \"yes\"}'

${YELLOW}Useful commands:${NC}
  # Check sequencer status
  curl http://localhost:$SEQUENCER_PORT/health

  # Check validator status
  curl http://localhost:$VALIDATOR_PORT/health

${YELLOW}Press Ctrl+C to stop all services${NC}
"
else
echo -e "
${GREEN}Components running:${NC}
  - Anvil (L1):     http://localhost:$ANVIL_PORT (PID: $ANVIL_PID)
  - Sequencer:      http://localhost:$SEQUENCER_PORT (PID: $SEQUENCER_PID)
  - Validator:      http://localhost:$VALIDATOR_PORT (PID: $VALIDATOR_PID)

${GREEN}Deployed contracts:${NC}
  - TestBridge:     $BRIDGE_ADDRESS

${GREEN}Data files:${NC}
  - Market DB:      $PM_DATABASE
  - Sequencer DB:   $DATA_DIR/sequencer.db
  - Validator DB:   $DATA_DIR/validator.db

${GREEN}Logs:${NC}
  - Sequencer:      $DATA_DIR/sequencer.log
  - Validator:      $DATA_DIR/validator.log

${YELLOW}Useful commands:${NC}
  # Check sequencer status
  curl http://localhost:$SEQUENCER_PORT/health

  # Check validator status
  curl http://localhost:$VALIDATOR_PORT/health

  # View sequencer batches
  curl http://localhost:$SEQUENCER_PORT/storage/batches

  # Get latest sequence
  curl http://localhost:$SEQUENCER_PORT/storage/latest

  # Run more prediction market commands
  ./target/release/prediction-market --db $PM_DATABASE --sequencer http://localhost:$SEQUENCER_PORT status

  # Start HTTP server instead
  ./target/release/prediction-market --db $PM_DATABASE --sequencer http://localhost:$SEQUENCER_PORT serve --port 8080

  # Emit more bridge events
  cast send $BRIDGE_ADDRESS \"deposit(address,uint256)\" 0x4444444444444444444444444444444444444444 100000000000000000000 --rpc-url http://localhost:$ANVIL_PORT --private-key $PRIVATE_KEY

  # View bridge events
  cast logs --address $BRIDGE_ADDRESS --rpc-url http://localhost:$ANVIL_PORT

${YELLOW}Press Ctrl+C to stop all services${NC}
"
fi

# Keep running until interrupted
wait
