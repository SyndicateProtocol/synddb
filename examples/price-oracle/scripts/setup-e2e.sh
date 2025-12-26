#!/bin/bash
set -e

# Price Oracle E2E Test Setup Script
# Deploys PriceOracle and configures it for testing with the Bridge

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
EXAMPLE_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
PROJECT_ROOT="$(cd "$EXAMPLE_ROOT/../.." && pwd)"
CONTRACTS_DIR="$PROJECT_ROOT/contracts"

# Anvil default accounts (deterministic)
ADMIN_ADDRESS="0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"
ADMIN_PRIVATE_KEY="0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
VALIDATOR_ADDRESS="0x70997970C51812dc3A010C7d01b50e0d17dc79C8"
VALIDATOR_PRIVATE_KEY="0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d"

RPC_URL="http://127.0.0.1:8545"

echo "========================================="
echo "Price Oracle E2E Test Setup"
echo "========================================="

# Check if Anvil is running
if ! curl -s $RPC_URL -X POST -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":1}' > /dev/null 2>&1; then
    echo "ERROR: Anvil is not running. Please start it first:"
    echo "  anvil --host 127.0.0.1 --port 8545 --chain-id 31337"
    exit 1
fi
echo "Anvil is running"

# Check if base E2E test is set up (Bridge deployed)
if [ -z "$BRIDGE_ADDRESS" ]; then
    if [ -f "$PROJECT_ROOT/.e2e-test-env" ]; then
        echo "Loading base E2E environment..."
        source "$PROJECT_ROOT/.e2e-test-env"
    else
        echo "ERROR: Bridge not deployed. Run the base E2E setup first:"
        echo "  ./scripts/setup-e2e-test.sh"
        exit 1
    fi
fi

echo "Bridge Address: $BRIDGE_ADDRESS"

cd "$CONTRACTS_DIR"

# Deploy PriceOracle
echo ""
echo "Deploying PriceOracle..."
ORACLE_OUTPUT=$(forge create test/use-cases/mocks/PriceOracle.sol:PriceOracle \
    --rpc-url $RPC_URL \
    --private-key $ADMIN_PRIVATE_KEY \
    --broadcast \
    --constructor-args $BRIDGE_ADDRESS 2>&1)

ORACLE_ADDRESS=$(echo "$ORACLE_OUTPUT" | grep "Deployed to:" | awk '{print $3}')
if [ -z "$ORACLE_ADDRESS" ]; then
    echo "Failed to deploy PriceOracle:"
    echo "$ORACLE_OUTPUT"
    exit 1
fi
echo "PriceOracle deployed to: $ORACLE_ADDRESS"

# Register updatePrice message type
echo ""
echo "Registering updatePrice message type..."
# schema_hash = 0x0 (no schema enforcement on-chain for now)
# schema_uri = "" (empty for now, could point to schemas/update-price.schema.json)
cast send $BRIDGE_ADDRESS "registerMessageType(string,address,bytes32,string)" \
    "updatePrice(string,uint256,uint256)" $ORACLE_ADDRESS \
    0x0000000000000000000000000000000000000000000000000000000000000000 "" \
    --rpc-url $RPC_URL \
    --private-key $ADMIN_PRIVATE_KEY > /dev/null

echo "Message type registered"

# Compute domain hash for price-oracle
ORACLE_DOMAIN=$(cast keccak "price-oracle")
echo ""
echo "Domain hash for 'price-oracle': $ORACLE_DOMAIN"

# Register price-oracle application
echo "Registering price-oracle application..."
cast send $BRIDGE_ADDRESS \
    "registerApplication(bytes32,(address,uint64,bool,bool))" \
    $ORACLE_DOMAIN "($VALIDATOR_ADDRESS,86400,true,true)" \
    --rpc-url $RPC_URL \
    --private-key $ADMIN_PRIVATE_KEY > /dev/null

echo "Application registered"

# Verify deployment
echo ""
echo "Verifying deployment..."
BRIDGE_FROM_ORACLE=$(cast call $ORACLE_ADDRESS "bridge()" --rpc-url $RPC_URL)
echo "Oracle.bridge() = $BRIDGE_FROM_ORACLE"

echo ""
echo "========================================="
echo "Price Oracle Setup Complete!"
echo "========================================="
echo ""
echo "PriceOracle Address: $ORACLE_ADDRESS"
echo "Bridge Address: $BRIDGE_ADDRESS"
echo "Domain: $ORACLE_DOMAIN (price-oracle)"
echo ""
echo "Message type: updatePrice(string,uint256,uint256)"
echo "Selector: $(cast sig 'updatePrice(string,uint256,uint256)')"
echo ""

# Save to environment file
cat > "$EXAMPLE_ROOT/.e2e-env" << EOF
export BRIDGE_ADDRESS=$BRIDGE_ADDRESS
export ORACLE_ADDRESS=$ORACLE_ADDRESS
export ORACLE_DOMAIN=$ORACLE_DOMAIN
export VALIDATOR_ADDRESS=$VALIDATOR_ADDRESS
export VALIDATOR_PRIVATE_KEY=$VALIDATOR_PRIVATE_KEY
export RPC_URL=$RPC_URL
EOF

echo "Environment saved to examples/price-oracle/.e2e-env"
echo ""
echo "To test the price oracle:"
echo "  source examples/price-oracle/.e2e-env"
echo "  cd examples/price-oracle"
echo "  source .venv/bin/activate"
echo "  pytest tests/ -v"
echo ""
echo "To test via Python client:"
echo "  python -c \""
echo "    import asyncio"
echo "    from price_oracle.bridge.client import BridgeClient"
echo "    client = BridgeClient('http://localhost:8080', '$ORACLE_DOMAIN')"
echo "    asyncio.run(client.push_price('bitcoin', 67196.645))"
echo "  \""
