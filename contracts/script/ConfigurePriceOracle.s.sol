// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {Script, console} from "forge-std/Script.sol";
import {PriceOracle} from "src/examples/PriceOracle.sol";

/**
 * @title ConfigurePriceOracle
 * @notice Configuration script for an existing PriceOracle contract
 * @dev Grants the UPDATER_ROLE to the Bridge contract.
 *
 * Required environment variables:
 *   - PRICE_ORACLE_ADDRESS: Address of the deployed PriceOracle contract
 *   - BRIDGE_CONTRACT_ADDRESS: Address of the Bridge contract to grant UPDATER_ROLE
 *
 * Usage:
 *   forge script script/ConfigurePriceOracle.s.sol \
 *     --rpc-url $RPC_URL \
 *     --broadcast \
 *     -vvvv
 */
contract ConfigurePriceOracle is Script {
    function run() external {
        address priceOracleAddress = vm.envAddress("PRICE_ORACLE_ADDRESS");
        address bridgeAddress = vm.envAddress("BRIDGE_CONTRACT_ADDRESS");

        PriceOracle oracle = PriceOracle(priceOracleAddress);
        bytes32 UPDATER_ROLE = oracle.UPDATER_ROLE();

        console.log("========================================");
        console.log("Configuring PriceOracle Contract");
        console.log("========================================");
        console.log("PriceOracle:", priceOracleAddress);
        console.log("Bridge:", bridgeAddress);
        console.log("UPDATER_ROLE:", vm.toString(UPDATER_ROLE));
        console.log("========================================");

        // Check current state
        bool bridgeHasRole = oracle.hasRole(UPDATER_ROLE, bridgeAddress);
        console.log("Bridge already has UPDATER_ROLE:", bridgeHasRole);

        if (bridgeHasRole) {
            console.log("No action needed - Bridge already has UPDATER_ROLE");
            return;
        }

        console.log("Granting UPDATER_ROLE to Bridge...");

        vm.startBroadcast();

        oracle.grantRole(UPDATER_ROLE, bridgeAddress);

        vm.stopBroadcast();

        // Verify
        bool success = oracle.hasRole(UPDATER_ROLE, bridgeAddress);
        require(success, "Failed to grant UPDATER_ROLE");

        console.log("========================================");
        console.log("Configuration Successful!");
        console.log("========================================");
        console.log("Bridge now has UPDATER_ROLE on PriceOracle");
        console.log("");
        console.log("The Bridge can now call:");
        console.log("  - updatePrice(string asset, uint256 price, uint256 timestamp)");
        console.log("  - fulfillPriceRequest(bytes32 requestId, string asset, uint256 price, uint256 timestamp)");
        console.log("  - batchUpdatePrices(string[] assets, uint256[] prices, uint256[] timestamps)");
        console.log("========================================");
    }
}
