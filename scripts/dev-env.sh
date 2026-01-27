#!/bin/bash
#
# SyndDB Local Development Environment
#
# Starts all components for local development:
#   1. Anvil (local Ethereum node)
#   2. Smart contracts (Bridge, MockWETH, PriceOracle)
#   3. SyndDB Sequencer
#   4. SyndDB Validator (optional)
#
# Usage:
#   ./scripts/dev-env.sh              # Start Anvil + contracts + sequencer
#   ./scripts/dev-env.sh --validator  # Also start validator
#   ./scripts/dev-env.sh --help       # Show help
#
# Press Ctrl-C to stop all components.

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# Load configuration from .env.defaults (single source of truth)
set -a
# shellcheck source=../.env.defaults
source "$PROJECT_ROOT/.env.defaults"
set +a

# Allow environment overrides for ports and data dir
ANVIL_PORT="${ANVIL_PORT:-8545}"
SEQUENCER_PORT="${SEQUENCER_PORT:-8433}"
VALIDATOR_PORT="${VALIDATOR_PORT:-8080}"
DATA_DIR="${DATA_DIR:-$PROJECT_ROOT/data}"

# Keys are loaded from .env.defaults (ANVIL_KEY_0, SEQUENCER_PUBKEY)

# Options
START_VALIDATOR=false
BUILD_FIRST=false
VERBOSE=false

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m'

log() { echo -e "${BLUE}[dev-env]${NC} $1"; }
success() { echo -e "${GREEN}[dev-env]${NC} $1"; }
warn() { echo -e "${YELLOW}[dev-env]${NC} $1"; }
error() { echo -e "${RED}[dev-env]${NC} $1"; }

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --validator|-v)
            START_VALIDATOR=true
            shift
            ;;
        --build|-b)
            BUILD_FIRST=true
            shift
            ;;
        --verbose)
            VERBOSE=true
            shift
            ;;
        --help|-h)
            echo "Usage: $0 [OPTIONS]"
            echo ""
            echo "Start SyndDB local development environment"
            echo ""
            echo "Options:"
            echo "  --validator, -v  Also start validator"
            echo "  --build, -b      Build before starting"
            echo "  --verbose        Show component output"
            echo "  --help, -h       Show this help"
            echo ""
            echo "Environment variables:"
            echo "  ANVIL_PORT       Anvil port (default: 8545)"
            echo "  SEQUENCER_PORT   Sequencer port (default: 8433)"
            echo "  VALIDATOR_PORT   Validator port (default: 8080)"
            echo "  DATA_DIR         Data directory (default: ./data)"
            exit 0
            ;;
        *)
            error "Unknown option: $1"
            exit 1
            ;;
    esac
done

# Track PIDs for cleanup
PIDS=()
cleanup() {
    echo ""
    log "Shutting down..."

    for pid in "${PIDS[@]}"; do
        if kill -0 "$pid" 2>/dev/null; then
            kill "$pid" 2>/dev/null || true
        fi
    done

    # Give processes time to exit gracefully
    sleep 1

    # Force kill if needed
    for pid in "${PIDS[@]}"; do
        if kill -0 "$pid" 2>/dev/null; then
            kill -9 "$pid" 2>/dev/null || true
        fi
    done

    success "All components stopped"
    exit 0
}
trap cleanup SIGINT SIGTERM

# Check dependencies
check_deps() {
    local missing=()

    command -v anvil >/dev/null 2>&1 || missing+=("anvil (install Foundry)")
    command -v forge >/dev/null 2>&1 || missing+=("forge (install Foundry)")

    if [[ ! -f "$PROJECT_ROOT/target/release/synddb-sequencer" ]]; then
        if [[ "$BUILD_FIRST" != "true" ]]; then
            missing+=("synddb-sequencer binary (run: cargo build -p synddb-sequencer --release)")
        fi
    fi

    if [[ "$START_VALIDATOR" == "true" ]] && [[ ! -f "$PROJECT_ROOT/target/release/synddb-validator" ]]; then
        if [[ "$BUILD_FIRST" != "true" ]]; then
            missing+=("synddb-validator binary (run: cargo build -p synddb-validator --release)")
        fi
    fi

    if [[ ${#missing[@]} -gt 0 ]]; then
        error "Missing dependencies:"
        for dep in "${missing[@]}"; do
            echo "  - $dep"
        done
        exit 1
    fi
}

# Build if requested
build_components() {
    if [[ "$BUILD_FIRST" == "true" ]]; then
        log "Building components..."
        cargo build -p synddb-sequencer -p synddb-validator --release
        success "Build complete"
    fi
}

# Start Anvil
start_anvil() {
    # Check if already running
    if curl -s -X POST "http://127.0.0.1:$ANVIL_PORT" -H "Content-Type: application/json" \
       --data '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}' >/dev/null 2>&1; then
        log "Anvil already running on port $ANVIL_PORT"
        return 0
    fi

    log "Starting Anvil on port $ANVIL_PORT..."

    if [[ "$VERBOSE" == "true" ]]; then
        anvil --port "$ANVIL_PORT" &
    else
        anvil --port "$ANVIL_PORT" --silent &
    fi
    PIDS+=($!)

    # Wait for Anvil
    for i in {1..30}; do
        if curl -s -X POST "http://127.0.0.1:$ANVIL_PORT" -H "Content-Type: application/json" \
           --data '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}' >/dev/null 2>&1; then
            success "Anvil started"
            return 0
        fi
        sleep 0.2
    done

    error "Anvil failed to start"
    exit 1
}

# Deploy contracts
deploy_contracts() {
    log "Deploying contracts..."

    if [[ "$VERBOSE" == "true" ]]; then
        "$SCRIPT_DIR/deploy-local.sh"
    else
        "$SCRIPT_DIR/deploy-local.sh" >/dev/null 2>&1
    fi

    # Read deployed addresses
    if [[ -f "$PROJECT_ROOT/.synddb/local-addresses.json" ]]; then
        BRIDGE_ADDRESS=$(jq -r '.contracts.bridge' "$PROJECT_ROOT/.synddb/local-addresses.json")
        success "Contracts deployed (Bridge: $BRIDGE_ADDRESS)"
    else
        error "Failed to get deployed addresses"
        exit 1
    fi
}

# Start sequencer
start_sequencer() {
    log "Starting sequencer on port $SEQUENCER_PORT..."

    mkdir -p "$DATA_DIR"

    if [[ "$VERBOSE" == "true" ]]; then
        SIGNING_KEY="$ANVIL_KEY_0" \
        BIND_ADDRESS="127.0.0.1:$SEQUENCER_PORT" \
        DATABASE_PATH="$DATA_DIR/sequencer.db" \
        "$PROJECT_ROOT/target/release/synddb-sequencer" &
    else
        SIGNING_KEY="$ANVIL_KEY_0" \
        BIND_ADDRESS="127.0.0.1:$SEQUENCER_PORT" \
        DATABASE_PATH="$DATA_DIR/sequencer.db" \
        "$PROJECT_ROOT/target/release/synddb-sequencer" 2>&1 | grep --line-buffered -E "(INFO|WARN|ERROR)" &
    fi
    PIDS+=($!)

    # Wait for sequencer
    for i in {1..30}; do
        if curl -s "http://127.0.0.1:$SEQUENCER_PORT/health" >/dev/null 2>&1; then
            success "Sequencer started"
            return 0
        fi
        sleep 0.2
    done

    error "Sequencer failed to start"
    exit 1
}

# Start validator
start_validator() {
    log "Starting validator on port $VALIDATOR_PORT..."

    mkdir -p "$DATA_DIR"

    if [[ "$VERBOSE" == "true" ]]; then
        SEQUENCER_PUBKEY="$SEQUENCER_PUBKEY" \
        SEQUENCER_URL="http://127.0.0.1:$SEQUENCER_PORT" \
        DATABASE_PATH="$DATA_DIR/validator.db" \
        STATE_DB_PATH="$DATA_DIR/validator_state.db" \
        PENDING_CHANGESETS_DB_PATH="$DATA_DIR/pending_changesets.db" \
        BIND_ADDRESS="127.0.0.1:$VALIDATOR_PORT" \
        "$PROJECT_ROOT/target/release/synddb-validator" &
    else
        SEQUENCER_PUBKEY="$SEQUENCER_PUBKEY" \
        SEQUENCER_URL="http://127.0.0.1:$SEQUENCER_PORT" \
        DATABASE_PATH="$DATA_DIR/validator.db" \
        STATE_DB_PATH="$DATA_DIR/validator_state.db" \
        PENDING_CHANGESETS_DB_PATH="$DATA_DIR/pending_changesets.db" \
        BIND_ADDRESS="127.0.0.1:$VALIDATOR_PORT" \
        "$PROJECT_ROOT/target/release/synddb-validator" 2>&1 | grep --line-buffered -E "(INFO|WARN|ERROR)" &
    fi
    PIDS+=($!)

    # Wait for validator
    for i in {1..30}; do
        if curl -s "http://127.0.0.1:$VALIDATOR_PORT/health" >/dev/null 2>&1; then
            success "Validator started"
            return 0
        fi
        sleep 0.2
    done

    error "Validator failed to start"
    exit 1
}

# Print status
print_status() {
    echo ""
    echo -e "${GREEN}════════════════════════════════════════════════════════════════${NC}"
    echo -e "${GREEN}  SyndDB Development Environment Running${NC}"
    echo -e "${GREEN}════════════════════════════════════════════════════════════════${NC}"
    echo ""
    echo "  Components:"
    echo "    Anvil:      http://127.0.0.1:$ANVIL_PORT"
    echo "    Sequencer:  http://127.0.0.1:$SEQUENCER_PORT"
    if [[ "$START_VALIDATOR" == "true" ]]; then
        echo "    Validator:  http://127.0.0.1:$VALIDATOR_PORT"
    fi
    echo ""
    echo "  Contracts:"
    if [[ -f "$PROJECT_ROOT/.synddb/local-addresses.json" ]]; then
        echo "    Bridge:       $(jq -r '.contracts.bridge' "$PROJECT_ROOT/.synddb/local-addresses.json")"
        echo "    MockWETH:     $(jq -r '.contracts.weth' "$PROJECT_ROOT/.synddb/local-addresses.json")"
        echo "    PriceOracle:  $(jq -r '.contracts.priceOracle' "$PROJECT_ROOT/.synddb/local-addresses.json")"
    fi
    echo ""
    echo "  Data directory: $DATA_DIR"
    echo ""
    echo -e "${CYAN}  Press Ctrl-C to stop all components${NC}"
    echo ""
}

# Main
main() {
    cd "$PROJECT_ROOT"

    check_deps
    build_components
    start_anvil
    deploy_contracts
    start_sequencer

    if [[ "$START_VALIDATOR" == "true" ]]; then
        start_validator
    fi

    print_status

    # Wait for any process to exit
    wait -n "${PIDS[@]}" 2>/dev/null || true

    # If we get here, a process exited unexpectedly
    error "A component exited unexpectedly"
    cleanup
}

main
