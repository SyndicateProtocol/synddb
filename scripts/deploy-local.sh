#!/bin/bash
#
# Deploy SyndDB contracts to local Anvil
#
# This is the simplest way to get started with SyndDB development.
# It starts Anvil (if not running) and deploys the Bridge contract.
#
# The deployed addresses are deterministic when using Anvil's default accounts,
# so you can rely on them across restarts (as long as you start fresh).
#
# Usage:
#   ./scripts/deploy-local.sh          # Start Anvil + deploy
#   ./scripts/deploy-local.sh --reset  # Kill existing Anvil, start fresh
#
# Outputs:
#   - Prints contract addresses to stdout
#   - Saves addresses to .synddb/local-addresses.json

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
CONTRACTS_DIR="$PROJECT_ROOT/contracts"

# Load configuration from .env.defaults (single source of truth)
set -a
# shellcheck source=../.env.defaults
source "$PROJECT_ROOT/.env.defaults"
set +a

# Map to script's expected variable names
ANVIL_ACCOUNT_0="$ANVIL_ADDRESS_0"
ANVIL_PRIVATE_KEY_0="0x$ANVIL_KEY_0"
ANVIL_ACCOUNT_1="$ANVIL_ADDRESS_1"
RPC_URL="$ANVIL_RPC_URL"

# Output directory
OUTPUT_DIR="$PROJECT_ROOT/.synddb"

# Colors
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
BLUE='\033[0;34m'
NC='\033[0m'

log() { echo -e "${BLUE}[deploy]${NC} $1"; }
success() { echo -e "${GREEN}[deploy]${NC} $1"; }
warn() { echo -e "${YELLOW}[deploy]${NC} $1"; }

# Parse arguments
RESET=false
while [[ $# -gt 0 ]]; do
    case $1 in
        --reset)
            RESET=true
            shift
            ;;
        --help|-h)
            echo "Usage: $0 [OPTIONS]"
            echo ""
            echo "Deploy SyndDB contracts to local Anvil"
            echo ""
            echo "Options:"
            echo "  --reset    Kill existing Anvil and start fresh"
            echo "  --help     Show this help"
            exit 0
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

# Check for required tools
command -v anvil >/dev/null 2>&1 || { echo "Error: anvil not found. Install Foundry first."; exit 1; }
command -v forge >/dev/null 2>&1 || { echo "Error: forge not found. Install Foundry first."; exit 1; }

# Reset if requested
if [[ "$RESET" == "true" ]]; then
    log "Killing existing Anvil processes..."
    pkill -f "anvil.*--port.*$ANVIL_PORT" 2>/dev/null || true
    sleep 1
fi

# Check if Anvil is already running
if curl -s -X POST "$RPC_URL" -H "Content-Type: application/json" \
   --data '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}' >/dev/null 2>&1; then
    log "Anvil already running on port $ANVIL_PORT"
else
    log "Starting Anvil on port $ANVIL_PORT..."
    anvil --port "$ANVIL_PORT" --silent &
    ANVIL_PID=$!

    # Wait for Anvil to be ready
    for i in {1..30}; do
        if curl -s -X POST "$RPC_URL" -H "Content-Type: application/json" \
           --data '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}' >/dev/null 2>&1; then
            break
        fi
        sleep 0.1
    done

    success "Anvil started (PID: $ANVIL_PID)"
fi

# Deploy contracts
log "Deploying contracts..."
cd "$CONTRACTS_DIR"

# Use Anvil account 0 as admin, account 1 as sequencer
DEPLOY_OUTPUT=$(ADMIN_ADDRESS="$ANVIL_ACCOUNT_0" \
    SEQUENCER_ADDRESS="$ANVIL_ACCOUNT_1" \
    forge script script/DeployLocalDevEnv.s.sol \
    --rpc-url "$RPC_URL" \
    --private-key "$ANVIL_PRIVATE_KEY_0" \
    --broadcast \
    -v 2>&1) || {
    echo "Deployment failed:"
    echo "$DEPLOY_OUTPUT"
    exit 1
}

# Parse addresses from deployment output
WETH_ADDRESS=$(echo "$DEPLOY_OUTPUT" | grep -o 'MockWETH deployed: 0x[0-9a-fA-F]*' | grep -o '0x[0-9a-fA-F]*' || echo "")
BRIDGE_ADDRESS=$(echo "$DEPLOY_OUTPUT" | grep -o 'Bridge deployed: 0x[0-9a-fA-F]*' | grep -o '0x[0-9a-fA-F]*' || echo "")
ORACLE_ADDRESS=$(echo "$DEPLOY_OUTPUT" | grep -o 'PriceOracle deployed: 0x[0-9a-fA-F]*' | grep -o '0x[0-9a-fA-F]*' || echo "")

if [[ -z "$BRIDGE_ADDRESS" ]]; then
    echo "Failed to parse deployed addresses from output:"
    echo "$DEPLOY_OUTPUT"
    exit 1
fi

# Create output directory
mkdir -p "$OUTPUT_DIR"

# Save addresses to JSON
cat > "$OUTPUT_DIR/local-addresses.json" <<EOF
{
  "network": "anvil",
  "chainId": 31337,
  "rpcUrl": "$RPC_URL",
  "deployer": "$ANVIL_ACCOUNT_0",
  "sequencer": "$ANVIL_ACCOUNT_1",
  "contracts": {
    "weth": "$WETH_ADDRESS",
    "bridge": "$BRIDGE_ADDRESS",
    "priceOracle": "$ORACLE_ADDRESS"
  }
}
EOF

cd "$PROJECT_ROOT"

# Print summary
echo ""
success "Deployment complete!"
echo ""
echo "  Network:      Anvil (localhost:$ANVIL_PORT)"
echo "  Chain ID:     31337"
echo ""
echo "  Contracts:"
echo "    MockWETH:     $WETH_ADDRESS"
echo "    Bridge:       $BRIDGE_ADDRESS"
echo "    PriceOracle:  $ORACLE_ADDRESS"
echo ""
echo "  Accounts:"
echo "    Admin:        $ANVIL_ACCOUNT_0"
echo "    Sequencer:    $ANVIL_ACCOUNT_1"
echo ""
echo "  Saved to:     $OUTPUT_DIR/local-addresses.json"
echo ""
echo "  Use these addresses in your .env or config:"
echo "    BRIDGE_CONTRACT=$BRIDGE_ADDRESS"
echo ""
