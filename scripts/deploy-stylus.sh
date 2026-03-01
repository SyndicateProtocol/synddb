#!/bin/bash
#
# Deploy the SyndDB Stylus Attestation Verifier to Arbitrum
#
# This script handles the full lifecycle:
#   1. Check  - Verify the contract compiles and fits within Stylus limits
#   2. Deploy - Submit the WASM bytecode and activate on-chain
#   3. Init   - Call initialize() and configure trusted JWKs, image digests, signers
#   4. Verify - Optionally verify the deployment via reproducible build
#
# Usage:
#   # Deploy to Arbitrum Sepolia (testnet)
#   ./scripts/deploy-stylus.sh \
#     --endpoint https://sepolia-rollup.arbitrum.io/rpc \
#     --private-key-path ./deployer-key.txt \
#     --expiration-tolerance 300
#
#   # Deploy to local Nitro devnode
#   ./scripts/deploy-stylus.sh \
#     --endpoint http://localhost:8547 \
#     --private-key 0xb6b15c8cb491557369f3c7d2c287b053eb229daa9c22138887752191c9520659
#
#   # Check only (no deployment)
#   ./scripts/deploy-stylus.sh --check-only --endpoint https://sepolia-rollup.arbitrum.io/rpc
#
#   # Export ABI only
#   ./scripts/deploy-stylus.sh --export-abi
#
#   # Initialize an already-deployed contract
#   ./scripts/deploy-stylus.sh \
#     --init-only \
#     --contract-address 0x1234...abcd \
#     --endpoint https://sepolia-rollup.arbitrum.io/rpc \
#     --private-key-path ./deployer-key.txt \
#     --expiration-tolerance 300
#
# Environment Variables (alternative to CLI flags):
#   STYLUS_RPC_URL          - RPC endpoint URL
#   STYLUS_PRIVATE_KEY      - Deployer private key (hex string)
#   STYLUS_PRIVATE_KEY_PATH - Path to file containing private key
#   EXPIRATION_TOLERANCE    - JWT expiration tolerance in seconds (default: 300)
#
# Post-deployment configuration (run separately via cast):
#   cast send <CONTRACT> "addTrustedJwk(bytes32,bytes32)" <KID_HASH> <KEY_MATERIAL_HASH>
#   cast send <CONTRACT> "addAllowedImageDigestHash(bytes32)" <DIGEST_HASH>
#   cast send <CONTRACT> "addTrustedImageSigner(address)" <SIGNER_ADDRESS>

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
CONTRACT_DIR="$PROJECT_ROOT/contracts/stylus/attestation-verifier"
OUTPUT_DIR="$PROJECT_ROOT/.synddb"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
BLUE='\033[0;34m'
NC='\033[0m'

log() { echo -e "${BLUE}[stylus]${NC} $1"; }
success() { echo -e "${GREEN}[stylus]${NC} $1"; }
warn() { echo -e "${YELLOW}[stylus]${NC} $1"; }
error() { echo -e "${RED}[stylus]${NC} $1" >&2; }

# Default values
ENDPOINT="${STYLUS_RPC_URL:-}"
PRIVATE_KEY="${STYLUS_PRIVATE_KEY:-}"
PRIVATE_KEY_PATH="${STYLUS_PRIVATE_KEY_PATH:-}"
EXPIRATION_TOLERANCE="${EXPIRATION_TOLERANCE:-300}"
CONTRACT_ADDRESS=""
CHECK_ONLY=false
EXPORT_ABI=false
INIT_ONLY=false
NO_ACTIVATE=false
NO_VERIFY=false
ESTIMATE_GAS=false
SKIP_INIT=false

usage() {
    echo "Usage: $0 [OPTIONS]"
    echo ""
    echo "Deploy the SyndDB Stylus Attestation Verifier to Arbitrum"
    echo ""
    echo "Options:"
    echo "  --endpoint URL            RPC endpoint (or set STYLUS_RPC_URL)"
    echo "  --private-key KEY         Deployer private key (hex)"
    echo "  --private-key-path PATH   Path to file containing private key"
    echo "  --expiration-tolerance N  JWT expiration tolerance in seconds (default: 300)"
    echo "  --contract-address ADDR   Contract address (for --init-only)"
    echo "  --check-only              Only run cargo stylus check, don't deploy"
    echo "  --export-abi              Export the contract ABI and exit"
    echo "  --init-only               Skip deployment, only initialize an existing contract"
    echo "  --skip-init               Deploy but don't call initialize()"
    echo "  --no-activate             Deploy without activating (activate separately later)"
    echo "  --no-verify               Skip Docker-based reproducible build verification"
    echo "  --estimate-gas            Estimate deployment gas cost without deploying"
    echo "  --help                    Show this help"
    exit 0
}

# Helper for flags that require a value
require_arg() {
    if [[ $# -lt 2 || "$2" == --* ]]; then
        error "$1 requires a value"
        exit 1
    fi
}

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --endpoint)
            require_arg "$@"
            ENDPOINT="$2"
            shift 2
            ;;
        --private-key)
            require_arg "$@"
            PRIVATE_KEY="$2"
            shift 2
            ;;
        --private-key-path)
            require_arg "$@"
            PRIVATE_KEY_PATH="$2"
            shift 2
            ;;
        --expiration-tolerance)
            require_arg "$@"
            EXPIRATION_TOLERANCE="$2"
            shift 2
            ;;
        --contract-address)
            require_arg "$@"
            CONTRACT_ADDRESS="$2"
            shift 2
            ;;
        --check-only)
            CHECK_ONLY=true
            shift
            ;;
        --export-abi)
            EXPORT_ABI=true
            shift
            ;;
        --init-only)
            INIT_ONLY=true
            shift
            ;;
        --skip-init)
            SKIP_INIT=true
            shift
            ;;
        --no-activate)
            NO_ACTIVATE=true
            shift
            ;;
        --no-verify)
            NO_VERIFY=true
            shift
            ;;
        --estimate-gas)
            ESTIMATE_GAS=true
            shift
            ;;
        --help|-h)
            usage
            ;;
        *)
            error "Unknown option: $1"
            exit 1
            ;;
    esac
done

# --- Validation helpers ---

require_endpoint() {
    if [[ -z "$ENDPOINT" ]]; then
        error "No RPC endpoint provided. Use --endpoint or set STYLUS_RPC_URL"
        exit 1
    fi
}

require_private_key() {
    if [[ -z "$PRIVATE_KEY" && -z "$PRIVATE_KEY_PATH" ]]; then
        error "No private key provided. Use --private-key or --private-key-path"
        exit 1
    fi
    if [[ -n "$PRIVATE_KEY_PATH" && ! -f "$PRIVATE_KEY_PATH" ]]; then
        error "Private key file not found: $PRIVATE_KEY_PATH"
        exit 1
    fi
}

require_tool() {
    local tool="$1"
    local install_hint="$2"
    if ! command -v "$tool" >/dev/null 2>&1; then
        error "Required tool not found: $tool"
        echo "  Install: $install_hint" >&2
        exit 1
    fi
}

require_cargo_stylus() {
    require_tool "cargo" "https://rustup.rs"
    if ! cargo stylus --version >/dev/null 2>&1; then
        error "Required tool not found: cargo-stylus"
        echo "  Install: cargo install --force cargo-stylus" >&2
        exit 1
    fi
}

# Build the key flag for cargo stylus deploy
build_key_arg() {
    if [[ -n "$PRIVATE_KEY" ]]; then
        echo "--private-key=$PRIVATE_KEY"
    elif [[ -n "$PRIVATE_KEY_PATH" ]]; then
        echo "--private-key-path=$PRIVATE_KEY_PATH"
    fi
}

# Get the raw private key value for cast commands
get_private_key() {
    if [[ -n "$PRIVATE_KEY" ]]; then
        echo "$PRIVATE_KEY"
    elif [[ -n "$PRIVATE_KEY_PATH" ]]; then
        tr -d '[:space:]' < "$PRIVATE_KEY_PATH"
    fi
}

# Extract a 20-byte Ethereum address from text.
# Filters for lines containing "address" to avoid matching tx hashes.
extract_address() {
    grep -i 'address' | grep -oi '0x[0-9a-fA-F]\{40\}' | head -1
}

# Write deployment info as valid JSON
write_deployment_json() {
    local address="$1"
    local chain_id="$2"
    local deploy_tx="${3:-}"

    # Ensure chain_id is a number, fallback to null for JSON validity
    if ! [[ "$chain_id" =~ ^[0-9]+$ ]]; then
        chain_id="null"
    fi

    # deploymentTx: use JSON null (unquoted) when empty, quoted string otherwise
    local deploy_tx_json="null"
    if [[ -n "$deploy_tx" ]]; then
        deploy_tx_json="\"$deploy_tx\""
    fi

    mkdir -p "$OUTPUT_DIR"
    cat > "$OUTPUT_DIR/stylus-deployment.json" <<EOF
{
  "contract": "StylusAttestationVerifier",
  "address": "$address",
  "chainId": $chain_id,
  "endpoint": "$ENDPOINT",
  "expirationTolerance": $EXPIRATION_TOLERANCE,
  "deploymentTx": $deploy_tx_json,
  "deployedAt": "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
}
EOF
}

# --- Export ABI mode ---
if [[ "$EXPORT_ABI" == "true" ]]; then
    require_cargo_stylus
    log "Exporting ABI..."
    cd "$CONTRACT_DIR"
    cargo stylus export-abi
    exit 0
fi

# --- Check-only mode ---
if [[ "$CHECK_ONLY" == "true" ]]; then
    require_endpoint
    require_cargo_stylus
    log "Checking contract against $ENDPOINT..."
    cd "$CONTRACT_DIR"
    cargo stylus check --endpoint="$ENDPOINT"
    success "Contract passed all Stylus checks"
    exit 0
fi

# --- Init-only mode ---
if [[ "$INIT_ONLY" == "true" ]]; then
    require_endpoint
    require_private_key
    if [[ -z "$CONTRACT_ADDRESS" ]]; then
        error "--contract-address is required with --init-only"
        exit 1
    fi
    require_tool "cast" "https://book.getfoundry.sh"
    CAST_KEY=$(get_private_key)

    log "Initializing contract at $CONTRACT_ADDRESS..."
    log "  Expiration tolerance: ${EXPIRATION_TOLERANCE}s"

    cast send \
        --rpc-url "$ENDPOINT" \
        --private-key "$CAST_KEY" \
        "$CONTRACT_ADDRESS" \
        "initialize(uint64)" \
        "$EXPIRATION_TOLERANCE"

    success "Contract initialized"

    # Verify initialization
    OWNER=$(cast call --rpc-url "$ENDPOINT" "$CONTRACT_ADDRESS" "owner()(address)")
    IS_INIT=$(cast call --rpc-url "$ENDPOINT" "$CONTRACT_ADDRESS" "isInitialized()(bool)")
    TOLERANCE=$(cast call --rpc-url "$ENDPOINT" "$CONTRACT_ADDRESS" "expirationTolerance()(uint64)")

    echo ""
    success "Contract state:"
    echo "  Owner:                $OWNER"
    echo "  Initialized:          $IS_INIT"
    echo "  Expiration tolerance: $TOLERANCE"
    echo ""
    echo "Next steps - configure the verifier:"
    echo "  # Add a trusted JWK (Google's signing key)"
    echo "  cast send --rpc-url $ENDPOINT --private-key \$KEY \\"
    echo "    $CONTRACT_ADDRESS 'addTrustedJwk(bytes32,bytes32)' \$KID_HASH \$KEY_MATERIAL_HASH"
    echo ""
    echo "  # Add an allowed container image digest"
    echo "  cast send --rpc-url $ENDPOINT --private-key \$KEY \\"
    echo "    $CONTRACT_ADDRESS 'addAllowedImageDigestHash(bytes32)' \$DIGEST_HASH"
    echo ""
    echo "  # Add a trusted image signer"
    echo "  cast send --rpc-url $ENDPOINT --private-key \$KEY \\"
    echo "    $CONTRACT_ADDRESS 'addTrustedImageSigner(address)' \$SIGNER_ADDRESS"
    exit 0
fi

# --- Deploy mode ---
require_endpoint
require_private_key
require_cargo_stylus
require_tool "cast" "https://book.getfoundry.sh"

log "Deploying SyndDB Stylus Attestation Verifier"
log "  Endpoint: $ENDPOINT"
echo ""

# Step 1: Check
log "Step 1/3: Checking contract..."
cd "$CONTRACT_DIR"
cargo stylus check --endpoint="$ENDPOINT"
success "Contract passed Stylus checks"
echo ""

# Step 2: Deploy
log "Step 2/3: Deploying contract..."

KEY_ARG=$(build_key_arg)
if [[ -z "$KEY_ARG" ]]; then
    error "Failed to build key argument"
    exit 1
fi

DEPLOY_ARGS=(
    "--endpoint=$ENDPOINT"
    "$KEY_ARG"
)

if [[ "$NO_ACTIVATE" == "true" ]]; then
    DEPLOY_ARGS+=("--no-activate")
fi

if [[ "$NO_VERIFY" == "true" ]]; then
    DEPLOY_ARGS+=("--no-verify")
fi

if [[ "$ESTIMATE_GAS" == "true" ]]; then
    log "Estimating deployment gas cost..."
    cargo stylus deploy "${DEPLOY_ARGS[@]}" --estimate-gas
    exit 0
fi

# Capture deploy output to extract the contract address
DEPLOY_OUTPUT=$(cargo stylus deploy "${DEPLOY_ARGS[@]}" 2>&1) || {
    error "Deployment failed:"
    echo "$DEPLOY_OUTPUT"
    exit 1
}

echo "$DEPLOY_OUTPUT"

# Extract contract address from lines containing "address" to avoid matching tx hashes.
# cargo stylus deploy outputs: "deployed code at address: 0x..." or "contract address: 0x..."
DEPLOYED_ADDRESS=$(echo "$DEPLOY_OUTPUT" | extract_address)

if [[ -z "$DEPLOYED_ADDRESS" ]]; then
    error "Could not parse deployed contract address from output"
    error "Check the output above for the address"
    exit 1
fi

# Extract deployment tx hash if available
DEPLOY_TX=$(echo "$DEPLOY_OUTPUT" | grep -oi 'deployment tx hash: 0x[0-9a-fA-F]\{64\}' | grep -oi '0x[0-9a-fA-F]\{64\}' || echo "")

success "Contract deployed at: $DEPLOYED_ADDRESS"
echo ""

# Step 3: Initialize
if [[ "$SKIP_INIT" == "true" ]]; then
    warn "Skipping initialization (--skip-init)"
else
    log "Step 3/3: Initializing contract..."
    log "  Expiration tolerance: ${EXPIRATION_TOLERANCE}s"

    CAST_KEY=$(get_private_key)

    cast send \
        --rpc-url "$ENDPOINT" \
        --private-key "$CAST_KEY" \
        "$DEPLOYED_ADDRESS" \
        "initialize(uint64)" \
        "$EXPIRATION_TOLERANCE"

    success "Contract initialized"
fi

echo ""

# Save deployment info
CHAIN_ID=$(cast chain-id --rpc-url "$ENDPOINT" 2>/dev/null || echo "unknown")
write_deployment_json "$DEPLOYED_ADDRESS" "$CHAIN_ID" "$DEPLOY_TX"

success "Deployment info saved to $OUTPUT_DIR/stylus-deployment.json"
echo ""

# Print summary
echo "================================================================"
success "Deployment complete!"
echo "================================================================"
echo ""
echo "  Contract:             StylusAttestationVerifier"
echo "  Address:              $DEPLOYED_ADDRESS"
echo "  Chain ID:             $CHAIN_ID"
echo "  Expiration tolerance: ${EXPIRATION_TOLERANCE}s"
if [[ -n "$DEPLOY_TX" ]]; then
    echo "  Deployment TX:        $DEPLOY_TX"
fi
echo ""
echo "  Saved to: $OUTPUT_DIR/stylus-deployment.json"
echo ""

if [[ -n "$DEPLOY_TX" ]]; then
    echo "To verify the deployment (requires Docker):"
    echo "  cd contracts/stylus/attestation-verifier"
    echo "  cargo stylus verify --deployment-tx=$DEPLOY_TX --endpoint=$ENDPOINT"
    echo ""
fi

echo "Next steps - configure the verifier:"
echo ""
echo "  1. Add trusted JWK(s) from Google's JWKS endpoint:"
echo "     cast send --rpc-url $ENDPOINT --private-key \$KEY \\"
echo "       $DEPLOYED_ADDRESS 'addTrustedJwk(bytes32,bytes32)' \$KID_HASH \$KEY_MATERIAL_HASH"
echo ""
echo "  2. Add allowed container image digest(s):"
echo "     cast send --rpc-url $ENDPOINT --private-key \$KEY \\"
echo "       $DEPLOYED_ADDRESS 'addAllowedImageDigestHash(bytes32)' \$DIGEST_HASH"
echo ""
echo "  3. Add trusted image signer(s):"
echo "     cast send --rpc-url $ENDPOINT --private-key \$KEY \\"
echo "       $DEPLOYED_ADDRESS 'addTrustedImageSigner(address)' \$SIGNER_ADDRESS"
echo ""
echo "  4. Update TeeKeyManager to use this verifier:"
echo "     cast send --rpc-url \$BASE_RPC --private-key \$KEY \\"
echo "       \$TEE_KEY_MANAGER 'updateAttestationVerifier(address)' $DEPLOYED_ADDRESS"
echo ""
