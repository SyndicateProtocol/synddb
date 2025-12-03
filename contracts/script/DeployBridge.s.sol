// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {Script} from "forge-std/Script.sol";
import {console} from "forge-std/console.sol";
import {Bridge} from "src/Bridge.sol";

/**
 * @title DeployBridge
 * @notice Deployment script for the Bridge contract to OP Stack networks
 * @dev Uses Foundry's script system with keystore-based signing.
 *      The wrapped native token address 0x4200000000000000000000000000000000000006 is consistent
 *      across all OP Stack networks (Optimism, Base, Zora, Mode, etc.) as it's a predeploy address.
 *      See: https://docs.optimism.io/chain/addresses#op-mainnet-l2
 */
contract DeployBridge is Script {
    // OP Stack predeploy WETH address (consistent across all OP Stack L2s)
    // This includes: Optimism, Base, Zora, Mode, Fraxtal, and other OP Stack chains
    address constant OP_STACK_WRAPPED_NATIVE_TOKEN = 0x4200000000000000000000000000000000000006;

    function run() external returns (Bridge) {
        return run(OP_STACK_WRAPPED_NATIVE_TOKEN); // change this if deploying to non-OP Stack networks
    }

    function run(address wrappedNativeToken) public returns (Bridge) {
        // Read admin address from environment variable
        address admin = vm.envAddress("ADMIN_ADDRESS");

        console.log("========================================");
        console.log("Deploying Bridge Contract");
        console.log("========================================");
        console.log("Admin:", admin);
        console.log("Wrapped Native Token:", wrappedNativeToken);
        console.log("========================================");

        // Start broadcasting transactions
        vm.startBroadcast();

        Bridge bridge = new Bridge(admin, wrappedNativeToken);

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
