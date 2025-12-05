# include .env file and export its env vars
# (-include to ignore error if it does not exist)
# Note that any unset variables here will wipe the variables if they are set in
# .zshrc or .bashrc. Make sure that the variables are set in .env, especially if
# you're running into issues with fork tests
-include .env

# deploy Bridge.sol to Base Sepolia
deploy-bridge-base-sepolia :; forge script script/DeployBridge.s.sol:DeployBridge --rpc-url base_sepolia --account deployer --etherscan-api-key ${ETHERSCAN_API_KEY} --verify --broadcast -vvv