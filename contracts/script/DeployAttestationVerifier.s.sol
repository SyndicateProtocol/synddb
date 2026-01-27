// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {Script} from "forge-std/Script.sol";
import {console} from "forge-std/console.sol";
import {AttestationVerifier} from "src/attestation/AttestationVerifier.sol";
import {TeeKeyManager} from "src/attestation/TeeKeyManager.sol";

/// @dev SP1 Verifier Gateway for Groth16 proofs (deterministic CREATE2, same on all networks)
/// See: https://github.com/succinctlabs/sp1-contracts/tree/main/contracts/deployments
address constant SP1_VERIFIER_GATEWAY_GROTH16 = 0x397A5f7f3dBd538f23DE225B51f532c34448dA9B;

/**
 * @title DeployAttestationVerifier
 * @notice Deployment script for TEE attestation verification contracts
 * @dev Deploys AttestationVerifier and TeeKeyManager for GCP Confidential Space.
 *      After running this script, you must call keyManager.setBridge(bridgeAddress)
 *      once the Bridge is deployed.
 */
contract DeployAttestationVerifier is Script {
    function run() external returns (AttestationVerifier, TeeKeyManager) {
        address verifier = vm.envOr("SP1_VERIFIER_CONTRACT_ADDRESS", SP1_VERIFIER_GATEWAY_GROTH16);
        bytes32 attestationVerifierVKey = vm.envBytes32("ATTESTATION_VERIFIER_VKEY");
        bytes32 expectedImageDigestHash = vm.envBytes32("EXPECTED_IMAGE_DIGEST_HASH");
        uint64 expirationTolerance = uint64(vm.envUint("EXPIRATION_TOLERANCE"));

        console.log("========================================");
        console.log("Deploying TEE Attestation Verifier");
        console.log("========================================");
        console.log("SP1 Verifier:", verifier);
        console.log("VKey:", vm.toString(attestationVerifierVKey));
        console.log("Expected Image Digest Hash:", vm.toString(expectedImageDigestHash));
        console.log("Expiration Tolerance:", expirationTolerance);
        console.log("========================================");

        vm.startBroadcast();

        AttestationVerifier attestationVerifier =
            new AttestationVerifier(verifier, attestationVerifierVKey, expectedImageDigestHash, expirationTolerance);

        TeeKeyManager keyManager = new TeeKeyManager(attestationVerifier);

        vm.stopBroadcast();

        console.log("========================================");
        console.log("Deployment Successful!");
        console.log("========================================");
        console.log("AttestationVerifier:", address(attestationVerifier));
        console.log("TeeKeyManager:", address(keyManager));
        console.log("========================================");
        console.log("");
        console.log("IMPORTANT: Complete setup after Bridge deployment:");
        console.log("1. Deploy Bridge with this TeeKeyManager address");
        console.log("2. Set Bridge on TeeKeyManager:");
        console.log("   keyManager.setBridge(<bridge_address>)");
        console.log("");
        console.log("3. Add trusted JWK hashes:");
        console.log("   attestationVerifier.addTrustedJwkHash(<jwk_hash>)");
        console.log("");
        console.log("4. Register TEE keys through Bridge:");
        console.log("   bridge.registerSequencerKey(<publicValues>, <proofBytes>)");
        console.log("========================================");

        return (attestationVerifier, keyManager);
    }
}
