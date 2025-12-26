// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {Script, console} from "forge-std/Script.sol";
import {Bridge} from "src/Bridge.sol";
import {PriceOracle} from "src/examples/PriceOracle.sol";
import {MockWETH} from "test/use-cases/mocks/MockWETH.sol";

/**
 * @title DeployLocalDevEnv
 * @notice Combined deployment script for local Anvil development environment
 * @dev Deploys MockWETH, Bridge, and PriceOracle with proper role configuration.
 *
 * Required environment variables:
 *   - ADMIN_ADDRESS: Address to receive admin roles (typically deployer)
 *   - SEQUENCER_ADDRESS: Address of the sequencer (receives MESSAGE_INITIALIZER_ROLE on Bridge)
 *
 * Usage with Anvil:
 *   anvil &
 *   forge script script/DeployLocalDevEnv.s.sol --rpc-url http://127.0.0.1:8545 --broadcast
 */
contract DeployLocalDevEnv is Script {
    // Role constants (must match Bridge/ModuleCheckRegistry)
    bytes32 public constant MESSAGE_INITIALIZER_ROLE = keccak256("MESSAGE_INITIALIZER_ROLE");

    function run() external returns (MockWETH weth, Bridge bridge, PriceOracle oracle) {
        address admin = vm.envAddress("ADMIN_ADDRESS");
        address sequencer = vm.envAddress("SEQUENCER_ADDRESS");

        console.log("========================================");
        console.log("Deploying Local Development Environment");
        console.log("========================================");
        console.log("Admin:", admin);
        console.log("Sequencer:", sequencer);
        console.log("========================================");

        vm.startBroadcast();

        // 1. Deploy MockWETH
        weth = new MockWETH();
        console.log("MockWETH deployed:", address(weth));

        // 2. Deploy Bridge with admin and WETH
        bridge = new Bridge(admin, address(weth));
        console.log("Bridge deployed:", address(bridge));

        // 3. Grant MESSAGE_INITIALIZER_ROLE to sequencer
        bridge.grantRole(MESSAGE_INITIALIZER_ROLE, sequencer);
        console.log("Granted MESSAGE_INITIALIZER_ROLE to sequencer");

        // 4. Deploy PriceOracle with admin and bridge
        oracle = new PriceOracle(admin, address(bridge));
        console.log("PriceOracle deployed:", address(oracle));

        vm.stopBroadcast();

        console.log("");
        console.log("========================================");
        console.log("Deployment Complete!");
        console.log("========================================");
        console.log("MockWETH:    ", address(weth));
        console.log("Bridge:      ", address(bridge));
        console.log("PriceOracle: ", address(oracle));
        console.log("========================================");
        console.log("");
        console.log("Role Configuration:");
        console.log("  Bridge:");
        console.log("    - Admin has DEFAULT_ADMIN_ROLE");
        console.log("    - Sequencer has MESSAGE_INITIALIZER_ROLE");
        console.log("  PriceOracle:");
        console.log("    - Admin has DEFAULT_ADMIN_ROLE and UPDATER_ROLE");
        console.log("    - Bridge has UPDATER_ROLE");
        console.log("========================================");
        console.log("");
        console.log("To update prices via Bridge:");
        console.log("  1. Sequencer calls bridge.initializeAndHandleMessage(...)");
        console.log("  2. Payload: abi.encodeCall(PriceOracle.updatePrice, (asset, price, timestamp))");
        console.log("  3. Target: PriceOracle address");
        console.log("========================================");

        return (weth, bridge, oracle);
    }
}
