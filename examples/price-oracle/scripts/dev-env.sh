#!/bin/bash
# Development environment for Price Oracle Example
#
# This script starts all components:
# 1. Anvil (local Ethereum node)
# 2. Smart contracts (Bridge, PriceOracle)
# 3. SyndDB Sequencer
# 4. Price Oracle Custom Validator
# 5. Python Price Oracle Fetcher
#
# Usage:
#   ./dev-env.sh                   # Start with consistent mock APIs (prices should sync)
#   ./dev-env.sh --divergent       # Start with divergent mock APIs (validator should reject)
#   ./dev-env.sh --real            # Start with real APIs (requires API keys)
#   ./dev-env.sh --no-anvil        # Skip Anvil and contract deployment
#   ./dev-env.sh --help            # Show help

set -e

# Configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"
EXAMPLE_DIR="$SCRIPT_DIR/.."
CONTRACTS_DIR="$PROJECT_ROOT/contracts"

# Ports
ANVIL_PORT=8545
SEQUENCER_PORT=8433
VALIDATOR_PORT=8080
APP_PORT=5000

# Data directories
DATA_DIR="$EXAMPLE_DIR/data"
SEQUENCER_DATA="$DATA_DIR/sequencer"
VALIDATOR_DATA="$DATA_DIR/validator"
APP_DATA="$DATA_DIR/app"
CONTRACTS_DATA="$DATA_DIR/contracts"

# Parse arguments
USE_MOCK=true
DIVERGENT=false
DIVERGENCE=5.0
FETCH_INTERVAL=10
USE_ANVIL=true

while [[ $# -gt 0 ]]; do
    case $1 in
        --divergent)
            DIVERGENT=true
            shift
            ;;
        --divergence)
            DIVERGENCE="$2"
            shift 2
            ;;
        --real)
            USE_MOCK=false
            shift
            ;;
        --interval)
            FETCH_INTERVAL="$2"
            shift 2
            ;;
        --no-anvil)
            USE_ANVIL=false
            shift
            ;;
        --help|-h)
            echo "Usage: $0 [OPTIONS]"
            echo ""
            echo "Options:"
            echo "  --divergent       Use divergent mock APIs (validator will reject)"
            echo "  --divergence N    Set divergence percentage (default: 5.0)"
            echo "  --real            Use real APIs (requires COINGECKO_API_KEY, CMC_API_KEY)"
            echo "  --interval N      Fetch interval in seconds (default: 10)"
            echo "  --no-anvil        Skip Anvil and contract deployment"
            echo "  --help, -h        Show this help"
            echo ""
            echo "Environment variables:"
            echo "  COINGECKO_API_KEY    CoinGecko API key (optional, free tier works)"
            echo "  CMC_API_KEY          CoinMarketCap API key (required for real mode)"
            exit 0
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

log_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

log_success() {
    echo -e "${GREEN}[OK]${NC} $1"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

log_contract() {
    echo -e "${CYAN}[CONTRACT]${NC} $1"
}

# Cleanup function
cleanup() {
    log_info "Shutting down..."

    if [[ -n "$FETCHER_PID" ]] && kill -0 "$FETCHER_PID" 2>/dev/null; then
        log_info "Stopping fetcher (PID: $FETCHER_PID)"
        kill "$FETCHER_PID" 2>/dev/null || true
    fi

    if [[ -n "$VALIDATOR_PID" ]] && kill -0 "$VALIDATOR_PID" 2>/dev/null; then
        log_info "Stopping validator (PID: $VALIDATOR_PID)"
        kill "$VALIDATOR_PID" 2>/dev/null || true
    fi

    if [[ -n "$SEQUENCER_PID" ]] && kill -0 "$SEQUENCER_PID" 2>/dev/null; then
        log_info "Stopping sequencer (PID: $SEQUENCER_PID)"
        kill "$SEQUENCER_PID" 2>/dev/null || true
    fi

    if [[ -n "$ANVIL_PID" ]] && kill -0 "$ANVIL_PID" 2>/dev/null; then
        log_info "Stopping Anvil (PID: $ANVIL_PID)"
        kill "$ANVIL_PID" 2>/dev/null || true
    fi

    wait 2>/dev/null || true
    log_success "All components stopped"
}

trap cleanup EXIT INT TERM

# Create data directories
mkdir -p "$SEQUENCER_DATA" "$VALIDATOR_DATA" "$APP_DATA" "$CONTRACTS_DATA"

# Build Rust binaries
log_info "Building Rust binaries..."
cd "$PROJECT_ROOT"
cargo build --release -p synddb-sequencer -p price-oracle-validator -p synddb-client --features ffi 2>&1 | tail -5
log_success "Rust binaries built"

# Generate sequencer keys if needed
SEQUENCER_KEY="$SEQUENCER_DATA/signing.key"
if [[ ! -f "$SEQUENCER_KEY" ]]; then
    log_info "Generating sequencer signing key..."
    # Generate a random 32-byte hex key
    openssl rand -hex 32 > "$SEQUENCER_KEY"
    log_success "Generated new signing key"
fi

SIGNING_KEY=$(cat "$SEQUENCER_KEY")

# Compute sequencer Ethereum address from private key using cast
# The sequencer signs messages with this key, so we need its address for Bridge roles
log_info "Computing sequencer Ethereum address..."
SEQUENCER_ETH_ADDRESS=$(cast wallet address --private-key "0x$SIGNING_KEY" 2>/dev/null || echo "")
if [[ -z "$SEQUENCER_ETH_ADDRESS" ]]; then
    log_warn "Could not compute sequencer address (cast not available?)"
    SEQUENCER_ETH_ADDRESS="0x0000000000000000000000000000000000000000"
fi
log_info "Sequencer Ethereum address: $SEQUENCER_ETH_ADDRESS"

# ============================================
# Start Anvil and deploy contracts
# ============================================
if [[ "$USE_ANVIL" == "true" ]]; then
    log_info "Starting Anvil on port $ANVIL_PORT..."

    # Use first Anvil default account as admin (has 10000 ETH)
    # Private key: 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80
    ANVIL_ADMIN_KEY="0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
    ANVIL_ADMIN_ADDRESS="0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"

    anvil --port $ANVIL_PORT --silent &
    ANVIL_PID=$!
    sleep 2

    if ! kill -0 "$ANVIL_PID" 2>/dev/null; then
        log_error "Anvil failed to start"
        exit 1
    fi
    log_success "Anvil started (PID: $ANVIL_PID)"

    # Deploy contracts
    log_info "Deploying smart contracts..."
    cd "$CONTRACTS_DIR"

    # Run deployment script with environment variables
    DEPLOY_OUTPUT=$(ADMIN_ADDRESS="$ANVIL_ADMIN_ADDRESS" \
        SEQUENCER_ADDRESS="$SEQUENCER_ETH_ADDRESS" \
        forge script script/DeployLocalDevEnv.s.sol \
        --rpc-url "http://127.0.0.1:$ANVIL_PORT" \
        --private-key "$ANVIL_ADMIN_KEY" \
        --broadcast \
        -v 2>&1)

    # Parse deployed addresses from output
    WETH_ADDRESS=$(echo "$DEPLOY_OUTPUT" | grep -o 'MockWETH deployed: 0x[0-9a-fA-F]*' | grep -o '0x[0-9a-fA-F]*' || echo "")
    BRIDGE_ADDRESS=$(echo "$DEPLOY_OUTPUT" | grep -o 'Bridge deployed: 0x[0-9a-fA-F]*' | grep -o '0x[0-9a-fA-F]*' || echo "")
    ORACLE_ADDRESS=$(echo "$DEPLOY_OUTPUT" | grep -o 'PriceOracle deployed: 0x[0-9a-fA-F]*' | grep -o '0x[0-9a-fA-F]*' || echo "")

    if [[ -z "$BRIDGE_ADDRESS" ]] || [[ -z "$ORACLE_ADDRESS" ]]; then
        log_error "Failed to parse deployed contract addresses"
        echo "$DEPLOY_OUTPUT"
        exit 1
    fi

    log_contract "MockWETH:    $WETH_ADDRESS"
    log_contract "Bridge:      $BRIDGE_ADDRESS"
    log_contract "PriceOracle: $ORACLE_ADDRESS"

    # Save addresses for reference
    cat > "$CONTRACTS_DATA/addresses.json" <<EOF
{
    "chainId": 31337,
    "rpcUrl": "http://127.0.0.1:$ANVIL_PORT",
    "admin": "$ANVIL_ADMIN_ADDRESS",
    "sequencer": "$SEQUENCER_ETH_ADDRESS",
    "weth": "$WETH_ADDRESS",
    "bridge": "$BRIDGE_ADDRESS",
    "priceOracle": "$ORACLE_ADDRESS"
}
EOF
    log_success "Contract addresses saved to $CONTRACTS_DATA/addresses.json"

    cd "$PROJECT_ROOT"
else
    log_warn "Skipping Anvil and contract deployment (--no-anvil)"
    BRIDGE_ADDRESS=""
    ORACLE_ADDRESS=""
fi

# Get the public key from the sequencer (we need to start it briefly)
log_info "Extracting sequencer public key..."
SEQUENCER_PUBKEY=$(cd "$PROJECT_ROOT" && \
    SIGNING_KEY="$SIGNING_KEY" \
    DATABASE_PATH="$SEQUENCER_DATA/sequencer.db" \
    BIND_ADDRESS="127.0.0.1:0" \
    timeout 2 ./target/release/synddb-sequencer 2>&1 | grep -o 'public_key=[0-9a-f]*' | cut -d= -f2 || true)

if [[ -z "$SEQUENCER_PUBKEY" ]]; then
    log_warn "Could not extract public key, using placeholder (sequencer will log it)"
    # Use a placeholder - in real usage, you'd get this from the sequencer logs
    SEQUENCER_PUBKEY="0000000000000000000000000000000000000000000000000000000000000000"
fi

log_info "Sequencer public key: ${SEQUENCER_PUBKEY:0:16}..."

# Start sequencer
log_info "Starting sequencer on port $SEQUENCER_PORT..."
cd "$PROJECT_ROOT"
RUST_LOG=info \
    SIGNING_KEY="$SIGNING_KEY" \
    DATABASE_PATH="$SEQUENCER_DATA/sequencer.db" \
    BIND_ADDRESS="127.0.0.1:$SEQUENCER_PORT" \
    ./target/release/synddb-sequencer &
SEQUENCER_PID=$!
sleep 2

if ! kill -0 "$SEQUENCER_PID" 2>/dev/null; then
    log_error "Sequencer failed to start"
    exit 1
fi
log_success "Sequencer started (PID: $SEQUENCER_PID)"

# Extract actual public key from sequencer logs
sleep 1
ACTUAL_PUBKEY=$(cat "$SEQUENCER_DATA/sequencer.db" 2>/dev/null | strings | grep -o '[0-9a-f]\{128\}' | head -1 || echo "$SEQUENCER_PUBKEY")
if [[ -n "$ACTUAL_PUBKEY" ]] && [[ "$ACTUAL_PUBKEY" != "$SEQUENCER_PUBKEY" ]]; then
    SEQUENCER_PUBKEY="$ACTUAL_PUBKEY"
    log_info "Updated public key from sequencer"
fi

# Start custom validator
log_info "Starting price oracle validator on port $VALIDATOR_PORT..."
RUST_LOG=info \
    SEQUENCER_PUBKEY="$SEQUENCER_PUBKEY" \
    SEQUENCER_URL="http://127.0.0.1:$SEQUENCER_PORT" \
    DATABASE_PATH="$VALIDATOR_DATA/validator.db" \
    STATE_DB_PATH="$VALIDATOR_DATA/validator_state.db" \
    BIND_ADDRESS="127.0.0.1:$VALIDATOR_PORT" \
    MAX_PRICE_DIFFERENCE_BPS=100 \
    ./target/release/price-oracle-validator &
VALIDATOR_PID=$!
sleep 2

if ! kill -0 "$VALIDATOR_PID" 2>/dev/null; then
    log_error "Validator failed to start"
    exit 1
fi
log_success "Validator started (PID: $VALIDATOR_PID)"

# Install Python dependencies if needed
log_info "Setting up Python environment..."
cd "$EXAMPLE_DIR"
if [[ ! -d "venv" ]]; then
    python3 -m venv venv
    source venv/bin/activate
    pip install -q -r app/requirements.txt
else
    source venv/bin/activate
fi
log_success "Python environment ready"

# Initialize database
log_info "Initializing price oracle database..."
python -m app.main --db "$APP_DATA/prices.db" init
log_success "Database initialized"

# Build mock API arguments
MOCK_ARGS=""
if [[ "$USE_MOCK" == "true" ]]; then
    if [[ "$DIVERGENT" == "true" ]]; then
        MOCK_ARGS="--divergent --divergence $DIVERGENCE"
        log_warn "Using DIVERGENT mock APIs - validator should REJECT changesets!"
    else
        MOCK_ARGS="--mock"
        log_info "Using consistent mock APIs - validator should ACCEPT changesets"
    fi
else
    log_info "Using real APIs"
fi

# Start fetcher daemon
log_info "Starting price fetcher (interval: ${FETCH_INTERVAL}s)..."
python -m app.fetcher \
    --db "$APP_DATA/prices.db" \
    --sequencer-url "http://127.0.0.1:$SEQUENCER_PORT" \
    --interval "$FETCH_INTERVAL" \
    $MOCK_ARGS \
    -v &
FETCHER_PID=$!
sleep 2

if ! kill -0 "$FETCHER_PID" 2>/dev/null; then
    log_error "Fetcher failed to start"
    exit 1
fi
log_success "Fetcher started (PID: $FETCHER_PID)"

# Print summary
echo ""
echo "=============================================="
echo "  Price Oracle Development Environment"
echo "=============================================="
echo ""
echo "Components running:"
if [[ "$USE_ANVIL" == "true" ]]; then
    echo "  Anvil:        http://127.0.0.1:$ANVIL_PORT (PID: $ANVIL_PID)"
fi
echo "  Sequencer:    http://127.0.0.1:$SEQUENCER_PORT (PID: $SEQUENCER_PID)"
echo "  Validator:    http://127.0.0.1:$VALIDATOR_PORT (PID: $VALIDATOR_PID)"
echo "  Fetcher:      PID: $FETCHER_PID"
echo ""
if [[ "$USE_ANVIL" == "true" ]]; then
    echo "Deployed Contracts:"
    echo "  Bridge:       $BRIDGE_ADDRESS"
    echo "  PriceOracle:  $ORACLE_ADDRESS"
    echo "  WETH:         $WETH_ADDRESS"
    echo ""
fi
echo "Data directories:"
echo "  Sequencer:    $SEQUENCER_DATA"
echo "  Validator:    $VALIDATOR_DATA"
echo "  Application:  $APP_DATA"
if [[ "$USE_ANVIL" == "true" ]]; then
    echo "  Contracts:    $CONTRACTS_DATA"
fi
echo ""
if [[ "$DIVERGENT" == "true" ]]; then
    echo -e "${YELLOW}Mode: DIVERGENT (validator should reject changesets)${NC}"
elif [[ "$USE_MOCK" == "true" ]]; then
    echo -e "${GREEN}Mode: CONSISTENT (validator should accept changesets)${NC}"
else
    echo "Mode: REAL APIs"
fi
echo ""
echo "Press Ctrl+C to stop all components"
echo "=============================================="
echo ""

# Wait for any component to exit
wait
