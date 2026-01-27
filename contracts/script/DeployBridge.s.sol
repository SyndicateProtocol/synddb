// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {Script} from "forge-std/Script.sol";
import {console} from "forge-std/console.sol";
import {Bridge} from "src/Bridge.sol";

/**
 * @title DeployBridge
 * @notice Deployment script for the Bridge contract to OP Stack networks
 * @dev Uses Foundry's script system with keystore-based signing.
 *      Requires TEE_KEY_MANAGER_ADDRESS to be set for sequencer signature verification.
 *
 */
contract DeployBridge is Script {
    // OP Stack predeploy WETH address (consistent across all OP Stack L2s)
    // This includes: Optimism, Base, Zora, Mode, Fraxtal, and other OP Stack chains
    // 0x4200000000000000000000000000000000000006;
    // Arbitrum chains use an arbitrary address for WETH, so ensure to set the correct one in that case.

    function run() external returns (Bridge) {
        address admin = vm.envAddress("ADMIN_ADDRESS");
        address wrappedNativeToken = vm.envAddress("WRAPPED_NATIVE_TOKEN_ADDRESS");
        address teeKeyManager = vm.envAddress("TEE_KEY_MANAGER_ADDRESS");

        console.log("========================================");
        console.log("Deploying Bridge Contract");
        console.log("========================================");
        console.log("Admin:", admin);
        console.log("Wrapped Native Token:", wrappedNativeToken);
        console.log("TEE Key Manager:", teeKeyManager);
        console.log("========================================");

        // Start broadcasting transactions
        vm.startBroadcast();

        Bridge bridge = new Bridge(admin, wrappedNativeToken, teeKeyManager);

        vm.stopBroadcast();

        console.log("========================================");
        console.log("Deployment Successful!");
        console.log("========================================");
        console.log("Bridge Contract:", address(bridge));
        console.log("========================================");
        console.log("");
        console.log("Next Steps:");
        console.log("1. Grant MESSAGE_INITIALIZER_ROLE to relayer addresses (if using relayers):");
        console.log("   bridge.grantRole(MESSAGE_INITIALIZER_ROLE, <relayer_address>)");
        console.log("");
        console.log("2. Grant VALIDATOR_ROLE to validator addresses:");
        console.log("   bridge.grantRole(VALIDATOR_ROLE, <validator_address>)");
        console.log("");
        console.log("3. Add validation modules (if needed):");
        console.log("   bridge.addPreModule(<module_address>)");
        console.log("   bridge.addPostModule(<module_address>)");
        console.log("");
        console.log("Note: Sequencer keys are verified via TeeKeyManager, not role grants.");
        console.log("========================================");

        return bridge;
    }
}
