# SyndDB Contracts Module
#
# Usage:
#   just contracts::build      # Build contracts
#   just contracts::test       # Run tests
#   just contracts::fmt        # Format code
#   just contracts::deploy-base-sepolia  # Deploy to Base Sepolia
#
# This module runs in the contracts/ directory by default.

# Build contracts
build:
    forge build

# Run contract tests
test:
    forge test -vvv

# Run contract tests with gas report
test-gas:
    forge test -vvv --gas-report

# Format Solidity code
fmt:
    forge fmt

# Check formatting
fmt-check:
    forge fmt --check

# Clean build artifacts
clean:
    forge clean

# Update dependencies
update:
    forge update

# Generate gas snapshots
snapshot:
    forge snapshot

# Run static analysis
analyze:
    forge inspect Bridge bytecode

# Deploy to local Anvil (use root `just deploy` instead for full setup)
[no-cd]
deploy-local:
    ./scripts/deploy-local.sh

# Deploy Bridge to Base Sepolia
deploy-base-sepolia:
    forge script script/DeployBridge.s.sol:DeployBridge \
        --rpc-url base_sepolia \
        --account deployer \
        --etherscan-api-key ${ETHERSCAN_API_KEY} \
        --verify \
        --broadcast \
        -vvv

# Verify contract on Etherscan
verify contract address:
    forge verify-contract {{address}} {{contract}} \
        --chain base-sepolia \
        --etherscan-api-key ${ETHERSCAN_API_KEY}
