// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {Script, console} from "forge-std/Script.sol";
import {TestBridge} from "src/TestBridge.sol";

/**
 * @title DeployTestBridge
 * @notice Deployment script for the TestBridge contract (local development only)
 * @dev Used with anvil for local testing of chain monitor integration
 */
contract DeployTestBridge is Script {
    function run() external returns (TestBridge) {
        console.log("========================================");
        console.log("Deploying TestBridge Contract (dev only)");
        console.log("========================================");

        vm.startBroadcast();
        TestBridge bridge = new TestBridge();
        vm.stopBroadcast();

        console.log("TestBridge deployed to:", address(bridge));
        console.log("========================================");

        return bridge;
    }
}
