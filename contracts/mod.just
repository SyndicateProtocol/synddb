# SyndDB Contracts Module
#
# Usage:
#   just contracts::build      # Build contracts
#   just contracts::test       # Run tests
#   just contracts::fmt        # Format code
#   just contracts::deploy-base-sepolia  # Deploy to Base Sepolia
#
# This module runs in the contracts/ directory by default.
# Use bash with strict mode

set shell := ["bash", "-euo", "pipefail", "-c"]

# Build contracts
[group('build')]
build:
    forge build

# Run contract tests
[group('test')]
test:
    forge test -vvv

# Run contract tests with gas report
[group('test')]
test-gas:
    forge test -vvv --gas-report

# Format Solidity code
[group('quality')]
fmt:
    forge fmt

# Check formatting
[group('quality')]
fmt-check:
    forge fmt --check

# Clean build artifacts
[group('cleanup')]
clean:
    forge clean

# Update dependencies
[group('deps')]
update:
    forge update

# Generate gas snapshots
[group('analysis')]
snapshot:
    forge snapshot

# Run static analysis
[group('analysis')]
analyze:
    forge inspect Bridge bytecode

# Deploy to local Anvil (use root `just deploy` instead for full setup)
[group('deploy')]
[no-cd]
[private]
_deploy-local:
    ./scripts/deploy-local.sh

# Deploy Bridge to Base Sepolia
# Requires: PRIVATE_KEY and ETHERSCAN_API_KEY in .env
[confirm('This will deploy to Base Sepolia testnet. Continue?')]
[group('deploy')]
deploy-base-sepolia:
    forge script script/DeployBridge.s.sol:DeployBridge \
        --rpc-url base_sepolia \
        --private-key ${PRIVATE_KEY} \
        --etherscan-api-key ${ETHERSCAN_API_KEY} \
        --verify \
        --broadcast \
        -vvv

# Verify contract on Etherscan
[group('deploy')]
verify contract address:
    forge verify-contract {{ address }} {{ contract }} \
        --chain base-sepolia \
        --etherscan-api-key ${ETHERSCAN_API_KEY}
