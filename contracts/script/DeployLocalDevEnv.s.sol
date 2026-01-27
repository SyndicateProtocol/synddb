// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {Script, console} from "forge-std/Script.sol";
import {Bridge} from "src/Bridge.sol";
import {PriceOracle} from "src/examples/PriceOracle.sol";
import {MockWETH} from "test/use-cases/mocks/MockWETH.sol";
import {MockAttestationVerifier} from "src/attestation/MockAttestationVerifier.sol";
import {TeeKeyManager} from "src/attestation/TeeKeyManager.sol";

/**
 * @title DeployLocalDevEnv
 * @notice Combined deployment script for local Anvil development environment
 * @dev Deploys MockWETH, Bridge, PriceOracle, and TEE attestation contracts with mock verification.
 *
 * Required environment variables:
 *   - ADMIN_ADDRESS: Address to receive admin roles (typically deployer)
 *   - SEQUENCER_ADDRESS: Address of the sequencer (will be granted message initializer permission)
 *
 * Optional environment variables:
 *   - DEPLOY_MOCK_ATTESTATION: Set to "true" to deploy mock attestation on non-Anvil chains
 *
 * Usage with Anvil:
 *   anvil &
 *   forge script script/DeployLocalDevEnv.s.sol --rpc-url http://127.0.0.1:8545 --broadcast
 *
 * TEE Key Registration (after sequencer starts):
 *   1. Get sequencer address: curl -s http://localhost:8433/status | jq -r '.signer_address'
 *   2. Register key: cast send $BRIDGE "registerKey(uint8,bytes,bytes)" 0 $(cast abi-encode "f(address)" $SIGNER_ADDRESS) 0x
 *      (0 = KeyType.Sequencer, 1 = KeyType.Validator)
 */
contract DeployLocalDevEnv is Script {
    // Anvil chain ID
    uint256 public constant ANVIL_CHAIN_ID = 31337;

    function run()
        external
        returns (
            MockWETH weth,
            Bridge bridge,
            PriceOracle oracle,
            MockAttestationVerifier attestationVerifier,
            TeeKeyManager keyManager
        )
    {
        address admin = vm.envAddress("ADMIN_ADDRESS");
        address sequencer = vm.envAddress("SEQUENCER_ADDRESS");

        // Check if mock attestation should be deployed
        bool deployMock = block.chainid == ANVIL_CHAIN_ID || vm.envOr("DEPLOY_MOCK_ATTESTATION", false);

        console.log("========================================");
        console.log("Deploying Local Development Environment");
        console.log("========================================");
        console.log("Admin:", admin);
        console.log("Sequencer:", sequencer);
        console.log("Chain ID:", block.chainid);
        console.log("Deploy Mock Attestation:", deployMock);
        console.log("========================================");

        if (!deployMock) {
            console.log("WARNING: Mock attestation not deployed on non-Anvil chain.");
            console.log("Set DEPLOY_MOCK_ATTESTATION=true to override.");
            revert("Mock attestation requires Anvil or explicit override");
        }

        vm.startBroadcast();

        // 1. Deploy MockWETH
        weth = new MockWETH();
        console.log("MockWETH deployed:", address(weth));

        // 2. Deploy MockAttestationVerifier (for TEE key registration without real proofs)
        attestationVerifier = new MockAttestationVerifier();
        console.log("MockAttestationVerifier deployed:", address(attestationVerifier));

        // 3. Deploy TeeKeyManager with mock verifier
        keyManager = new TeeKeyManager(attestationVerifier);
        console.log("TeeKeyManager deployed:", address(keyManager));

        // 4. Deploy Bridge with admin, WETH, and TeeKeyManager
        bridge = new Bridge(admin, address(weth), address(keyManager));
        console.log("Bridge deployed:", address(bridge));

        // 5. Set Bridge on TeeKeyManager (required for key management)
        keyManager.setBridge(address(bridge));
        console.log("TeeKeyManager bridge set");

        // 6. Grant message initializer permission to sequencer
        bridge.setMessageInitializer(sequencer, true);
        console.log("Granted message initializer to sequencer");

        // 7. Deploy PriceOracle with admin and bridge
        oracle = new PriceOracle(admin, address(bridge));
        console.log("PriceOracle deployed:", address(oracle));

        vm.stopBroadcast();

        console.log("");
        console.log("========================================");
        console.log("Deployment Complete!");
        console.log("========================================");
        console.log("MockWETH:               ", address(weth));
        console.log("Bridge:                 ", address(bridge));
        console.log("PriceOracle:            ", address(oracle));
        console.log("MockAttestationVerifier:", address(attestationVerifier));
        console.log("TeeKeyManager:          ", address(keyManager));
        console.log("========================================");
        console.log("");
        console.log("Permission Configuration:");
        console.log("  Bridge:");
        console.log("    - Admin is owner (can manage keys, modules, permissions)");
        console.log("    - Sequencer can initialize messages");
        console.log("  PriceOracle:");
        console.log("    - Admin has DEFAULT_ADMIN_ROLE and UPDATER_ROLE");
        console.log("    - Bridge has UPDATER_ROLE");
        console.log("========================================");
        console.log("");
        console.log("TEE Key Registration (after sequencer starts):");
        console.log("  1. Get sequencer's dynamically generated address:");
        console.log("     SIGNER=$(curl -s http://localhost:8433/status | jq -r '.signer_address')");
        console.log("");
        console.log("  2. Register the key through Bridge (0 = Sequencer, 1 = Validator):");
        console.log("     cast send", address(bridge), "\"registerKey(uint8,bytes,bytes)\" 0 \\");
        console.log("       $(cast abi-encode \"f(address)\" $SIGNER) 0x --private-key $ANVIL_KEY_0");
        console.log("========================================");

        return (weth, bridge, oracle, attestationVerifier, keyManager);
    }
}
