// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {Script, console} from "forge-std/Script.sol";
import {PriceOracle} from "src/examples/PriceOracle.sol";

/**
 * @title DeployPriceOracle
 * @notice Deployment script for the PriceOracle example contract
 * @dev Deploys PriceOracle with admin and bridge addresses configured.
 *
 * Required environment variables:
 *   - ADMIN_ADDRESS: Address to receive admin role
 *   - BRIDGE_CONTRACT_ADDRESS: Address of the Bridge contract (receives UPDATER_ROLE)
 */
contract DeployPriceOracle is Script {
    function run() external returns (PriceOracle) {
        address admin = vm.envAddress("ADMIN_ADDRESS");
        address bridge = vm.envAddress("BRIDGE_CONTRACT_ADDRESS");

        console.log("========================================");
        console.log("Deploying PriceOracle Contract");
        console.log("========================================");
        console.log("Admin:", admin);
        console.log("Bridge:", bridge);
        console.log("========================================");

        vm.startBroadcast();

        PriceOracle oracle = new PriceOracle(admin, bridge);

        vm.stopBroadcast();

        console.log("========================================");
        console.log("Deployment Successful!");
        console.log("========================================");
        console.log("PriceOracle Contract:", address(oracle));
        console.log("========================================");
        console.log("");
        console.log("Configuration:");
        console.log("  - Admin has DEFAULT_ADMIN_ROLE and UPDATER_ROLE");
        console.log("  - Bridge has UPDATER_ROLE (can call updatePrice)");
        console.log("");
        console.log("The off-chain price fetcher can now:");
        console.log("  1. Submit price updates via Bridge.initializeAndHandleMessage()");
        console.log("  2. Listen for PriceRequested events and respond");
        console.log("========================================");

        return oracle;
    }
}
