#!/bin/bash
set -e

# E2E Test Setup Script
# Deploys MessageBridge to Anvil and configures it for testing

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
CONTRACTS_DIR="$PROJECT_ROOT/contracts"

# Anvil default accounts (deterministic)
ADMIN_ADDRESS="0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"
ADMIN_PRIVATE_KEY="0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
VALIDATOR_ADDRESS="0x70997970C51812dc3A010C7d01b50e0d17dc79C8"
VALIDATOR_PRIVATE_KEY="0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d"
WITNESS_ADDRESS="0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC"
WITNESS_PRIVATE_KEY="0x5de4111afa1a4b94908f83103eb1f1706367c2e68ca870fc3fb9a804cdab365a"

# WETH address on Anvil (we'll deploy a mock)
WETH_ADDRESS=""

# Bridge address (set after deployment)
BRIDGE_ADDRESS=""

echo "========================================="
echo "E2E Test Setup"
echo "========================================="

# Check if Anvil is running
if ! curl -s http://127.0.0.1:8545 -X POST -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":1}' > /dev/null 2>&1; then
    echo "Starting Anvil..."
    anvil --host 127.0.0.1 --port 8545 --chain-id 31337 &
    ANVIL_PID=$!
    sleep 2
    echo "Anvil started with PID $ANVIL_PID"
else
    echo "Anvil already running"
fi

cd "$CONTRACTS_DIR"

# Deploy WETH mock first
echo ""
echo "Deploying MockWETH..."
WETH_OUTPUT=$(forge create test/use-cases/mocks/MockWETH.sol:MockWETH \
    --rpc-url http://127.0.0.1:8545 \
    --private-key $ADMIN_PRIVATE_KEY \
    --broadcast 2>&1)

WETH_ADDRESS=$(echo "$WETH_OUTPUT" | grep "Deployed to:" | awk '{print $3}')
if [ -z "$WETH_ADDRESS" ]; then
    echo "Failed to deploy MockWETH:"
    echo "$WETH_OUTPUT"
    exit 1
fi
echo "MockWETH deployed to: $WETH_ADDRESS"

# Deploy MessageBridge
echo ""
echo "Deploying MessageBridge..."
BRIDGE_OUTPUT=$(forge create src/MessageBridge.sol:MessageBridge \
    --rpc-url http://127.0.0.1:8545 \
    --private-key $ADMIN_PRIVATE_KEY \
    --broadcast \
    --constructor-args $ADMIN_ADDRESS $WETH_ADDRESS 2 2>&1)

BRIDGE_ADDRESS=$(echo "$BRIDGE_OUTPUT" | grep "Deployed to:" | awk '{print $3}')
if [ -z "$BRIDGE_ADDRESS" ]; then
    echo "Failed to deploy MessageBridge:"
    echo "$BRIDGE_OUTPUT"
    exit 1
fi
echo "MessageBridge deployed to: $BRIDGE_ADDRESS"

# Deploy MockTarget for testing
echo ""
echo "Deploying MockTarget..."
MOCK_OUTPUT=$(forge create test/mocks/MockTarget.sol:MockTarget \
    --rpc-url http://127.0.0.1:8545 \
    --private-key $ADMIN_PRIVATE_KEY \
    --broadcast 2>&1)

MOCK_TARGET=$(echo "$MOCK_OUTPUT" | grep "Deployed to:" | awk '{print $3}')
if [ -z "$MOCK_TARGET" ]; then
    echo "Failed to deploy MockTarget:"
    echo "$MOCK_OUTPUT"
    exit 1
fi
echo "MockTarget deployed to: $MOCK_TARGET"

# Configure the bridge
echo ""
echo "Configuring MessageBridge..."

# Add witness validators
echo "Adding witness validators..."
cast send $BRIDGE_ADDRESS "addWitnessValidator(address,bytes)" $VALIDATOR_ADDRESS 0x \
    --rpc-url http://127.0.0.1:8545 \
    --private-key $ADMIN_PRIVATE_KEY > /dev/null

cast send $BRIDGE_ADDRESS "addWitnessValidator(address,bytes)" $WITNESS_ADDRESS 0x \
    --rpc-url http://127.0.0.1:8545 \
    --private-key $ADMIN_PRIVATE_KEY > /dev/null

# Register message type
echo "Registering message type..."
cast send $BRIDGE_ADDRESS "registerMessageType(string,address,bytes32,string)" \
    "setValue(uint256)" $MOCK_TARGET 0x0000000000000000000000000000000000000000000000000000000000000000 "" \
    --rpc-url http://127.0.0.1:8545 \
    --private-key $ADMIN_PRIVATE_KEY > /dev/null

# Compute domain hash for test-app
DOMAIN=$(cast keccak "test-app")
echo "Domain hash for 'test-app': $DOMAIN"

# Register application
echo "Registering application..."
cast send $BRIDGE_ADDRESS \
    "registerApplication(bytes32,(address,uint64,bool,bool))" \
    $DOMAIN "($VALIDATOR_ADDRESS,86400,true,true)" \
    --rpc-url http://127.0.0.1:8545 \
    --private-key $ADMIN_PRIVATE_KEY > /dev/null

echo ""
echo "========================================="
echo "Setup Complete!"
echo "========================================="
echo ""
echo "Bridge Address: $BRIDGE_ADDRESS"
echo "WETH Address: $WETH_ADDRESS"
echo "MockTarget Address: $MOCK_TARGET"
echo ""
echo "Validators:"
echo "  Primary: $VALIDATOR_ADDRESS"
echo "  Witness: $WITNESS_ADDRESS"
echo ""
echo "Domain: $DOMAIN (test-app)"
echo ""
echo "To run E2E tests:"
echo "  export BRIDGE_ADDRESS=$BRIDGE_ADDRESS"
echo "  cargo test -p synddb-bridge-validator --test e2e_test -- --ignored"
echo ""

# Save addresses to file for convenience
cat > "$PROJECT_ROOT/.e2e-test-env" << EOF
export BRIDGE_ADDRESS=$BRIDGE_ADDRESS
export WETH_ADDRESS=$WETH_ADDRESS
export MOCK_TARGET=$MOCK_TARGET
export DOMAIN=$DOMAIN
export VALIDATOR_ADDRESS=$VALIDATOR_ADDRESS
export VALIDATOR_PRIVATE_KEY=$VALIDATOR_PRIVATE_KEY
export WITNESS_ADDRESS=$WITNESS_ADDRESS
export WITNESS_PRIVATE_KEY=$WITNESS_PRIVATE_KEY
EOF

echo "Environment saved to .e2e-test-env"
echo "Run: source .e2e-test-env && cargo test -p synddb-bridge-validator --test e2e_test -- --ignored"
