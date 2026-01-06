// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {Script} from "forge-std/Script.sol";
import {console} from "forge-std/console.sol";
import {AttestationVerifier} from "src/attestation/AttestationVerifier.sol";
import {TeeKeyManager} from "src/attestation/TeeKeyManager.sol";

/// @dev SP1 Verifier Gateway for Groth16 proofs (deterministic CREATE2, same on all networks)
/// See: https://github.com/succinctlabs/sp1-contracts/tree/main/contracts/deployments
address constant SP1_VERIFIER_GATEWAY_GROTH16 = 0x397A5f7f3dBd538f23DE225B51f532c34448dA9B;

/// @dev Default trusted JWK hashes for GCP Confidential Computing
/// These are keccak256 hashes of Google's attestation signing keys from:
/// https://confidentialcomputing.googleapis.com/v1alpha1/jwks
/// Keys may rotate, so additional hashes can be added via TRUSTED_JWK_HASHES env var.
string constant DEFAULT_TRUSTED_JWK_HASHES = "0xc4339aa224c54c5dcad4bf4d0183fd5a7d4eb346b3064b0c3ea938c415b19b5f";

/**
 * @title DeployAttestationVerifier
 * @notice Deployment script for TEE attestation verification contracts
 * @dev Deploys AttestationVerifier and TeeKeyManager for GCP Confidential Space.
 *      After running this script, you must call keyManager.setBridge(bridgeAddress)
 *      once the Bridge is deployed.
 *
 *      Optional: Set TRUSTED_JWK_HASHES to a comma-separated list of JWK hashes
 *      to add during deployment. These are the keccak256 hashes of Google's
 *      Confidential Computing attestation signing keys.
 */
contract DeployAttestationVerifier is Script {
    function run() external returns (AttestationVerifier, TeeKeyManager) {
        address verifier = vm.envOr("SP1_VERIFIER_CONTRACT_ADDRESS", SP1_VERIFIER_GATEWAY_GROTH16);
        bytes32 attestationVerifierVKey = vm.envBytes32("ATTESTATION_VERIFIER_VKEY");
        bytes32 expectedImageDigestHash = vm.envBytes32("EXPECTED_IMAGE_DIGEST_HASH");
        uint64 expirationTolerance = uint64(vm.envUint("EXPIRATION_TOLERANCE"));
        string memory trustedJwkHashesStr = vm.envOr("TRUSTED_JWK_HASHES", DEFAULT_TRUSTED_JWK_HASHES);

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

        // Add trusted JWK hashes if provided
        if (bytes(trustedJwkHashesStr).length > 0) {
            bytes32[] memory jwkHashes = _parseJwkHashes(trustedJwkHashesStr);
            for (uint256 i = 0; i < jwkHashes.length; i++) {
                attestationVerifier.addTrustedJwkHash(jwkHashes[i]);
                console.log("Added trusted JWK hash:", vm.toString(jwkHashes[i]));
            }
        }

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

    /// @dev Parse comma-separated hex strings into bytes32 array
    function _parseJwkHashes(string memory input) internal pure returns (bytes32[] memory) {
        // Count commas to determine array size
        bytes memory inputBytes = bytes(input);
        uint256 count = 1;
        for (uint256 i = 0; i < inputBytes.length; i++) {
            if (inputBytes[i] == ",") {
                count++;
            }
        }

        bytes32[] memory result = new bytes32[](count);
        uint256 start = 0;
        uint256 idx = 0;

        for (uint256 i = 0; i <= inputBytes.length; i++) {
            if (i == inputBytes.length || inputBytes[i] == ",") {
                // Extract substring
                bytes memory hashBytes = new bytes(i - start);
                for (uint256 j = start; j < i; j++) {
                    hashBytes[j - start] = inputBytes[j];
                }
                result[idx] = _hexToBytes32(string(hashBytes));
                idx++;
                start = i + 1;
            }
        }

        return result;
    }

    /// @dev Convert hex string to bytes32 (with or without 0x prefix)
    function _hexToBytes32(string memory s) internal pure returns (bytes32) {
        bytes memory ss = bytes(s);
        uint256 start = 0;

        // Skip 0x prefix if present
        if (ss.length >= 2 && ss[0] == "0" && (ss[1] == "x" || ss[1] == "X")) {
            start = 2;
        }

        require(ss.length - start == 64, "Invalid bytes32 hex string length");

        bytes32 result;
        for (uint256 i = 0; i < 32; i++) {
            uint8 high = _hexCharToUint8(ss[start + i * 2]);
            uint8 low = _hexCharToUint8(ss[start + i * 2 + 1]);
            result = bytes32(uint256(result) | (uint256(high * 16 + low) << (8 * (31 - i))));
        }

        return result;
    }

    /// @dev Convert hex character to uint8
    function _hexCharToUint8(bytes1 c) internal pure returns (uint8) {
        if (c >= "0" && c <= "9") return uint8(c) - uint8(bytes1("0"));
        if (c >= "a" && c <= "f") return uint8(c) - uint8(bytes1("a")) + 10;
        if (c >= "A" && c <= "F") return uint8(c) - uint8(bytes1("A")) + 10;
        revert("Invalid hex character");
    }
}
