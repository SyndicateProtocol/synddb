#!/bin/bash
#
# Well-known Anvil addresses for SyndDB development
#
# Source this file to get consistent addresses:
#   source scripts/anvil-addresses.sh
#
# These addresses are deterministic when deploying with Anvil's default accounts.
# The contract addresses assume deployment from Account 0 with fresh nonces (0, 1, 2).

# Anvil default accounts (same as Hardhat)
export ANVIL_ACCOUNT_0="0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"
export ANVIL_ACCOUNT_1="0x70997970C51812dc3A010C7d01b50e0d17dc79C8"
export ANVIL_ACCOUNT_2="0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC"

# Private keys (DO NOT USE IN PRODUCTION - these are well-known test keys)
export ANVIL_PRIVATE_KEY_0="0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
export ANVIL_PRIVATE_KEY_1="0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d"
export ANVIL_PRIVATE_KEY_2="0x5de4111afa1a4b94908f83103eb1f1706367c2e68ca870fc3fb9a804cdab365a"

# Deterministic contract addresses from DeployLocalDevEnv.s.sol
# (Nonce 2 is used by grantRole, so PriceOracle is at nonce 3)
export WETH_ADDRESS="0x5FbDB2315678afecb367f032d93F642f64180aa3"      # Nonce 0
export BRIDGE_ADDRESS="0xe7f1725E7734CE288F8367e1Bb143E90bb3F0512"    # Nonce 1
export ORACLE_ADDRESS="0xCf7Ed3AccA5a467e9e704C703E8D87F634fB0Fc9"    # Nonce 3

# Chain ID
export ANVIL_CHAIN_ID="31337"

# RPC URL
export ANVIL_RPC_URL="http://127.0.0.1:8545"

# For SyndDB components
export ADMIN_ADDRESS="$ANVIL_ACCOUNT_0"
export SEQUENCER_ADDRESS="$ANVIL_ACCOUNT_1"
export BRIDGE_CONTRACT="$BRIDGE_ADDRESS"
export BRIDGE_CHAIN_ID="$ANVIL_CHAIN_ID"
