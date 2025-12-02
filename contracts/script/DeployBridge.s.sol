// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {Script} from "forge-std/Script.sol";
import {console} from "forge-std/console.sol";
import {Bridge} from "src/Bridge.sol";

/**
 * @title DeployBridge
 * @notice Deployment script for the Bridge contract to Base Sepolia
 * @dev Uses Foundry's script system with keystore-based signing
 *      Wrapped Native Token (WETH) hardcoded for Base Sepolia: 0x4200000000000000000000000000000000000006
 */
contract DeployBridge is Script {
    // Base Sepolia WETH address (constant across all Base networks)
    address constant WRAPPED_NATIVE_TOKEN = 0x4200000000000000000000000000000000000006;

    function run() external returns (Bridge) {
        // Read admin address from environment variable
        address admin = vm.envAddress("ADMIN_ADDRESS");

        console.log("========================================");
        console.log("Deploying Bridge Contract to Base Sepolia");
        console.log("========================================");
        console.log("Admin:", admin);
        console.log("Wrapped Native Token (WETH):", WRAPPED_NATIVE_TOKEN);
        console.log("========================================");

        // Start broadcasting transactions
        vm.startBroadcast();

        Bridge bridge = new Bridge(admin, WRAPPED_NATIVE_TOKEN);

        vm.stopBroadcast();

        console.log("========================================");
        console.log("Deployment Successful!");
        console.log("========================================");
        console.log("Bridge Contract:", address(bridge));
        console.log("========================================");
        console.log("");
        console.log("Next Steps:");
        console.log("1. Grant SEQUENCER_ROLE to sequencer address:");
        console.log("   bridge.grantRole(SEQUENCER_ROLE, <sequencer_address>)");
        console.log("");
        console.log("2. Grant VALIDATOR_ROLE to validator addresses:");
        console.log("   bridge.grantRole(VALIDATOR_ROLE, <validator_address>)");
        console.log("");
        console.log("3. Add validation modules (if needed):");
        console.log("   bridge.addPreModule(<module_address>)");
        console.log("   bridge.addPostModule(<module_address>)");
        console.log("========================================");

        return bridge;
    }
}
