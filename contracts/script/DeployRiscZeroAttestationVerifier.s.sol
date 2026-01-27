// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {Script} from "forge-std/Script.sol";
import {console} from "forge-std/console.sol";
import {RiscZeroAttestationVerifier} from "src/attestation/RiscZeroAttestationVerifier.sol";
import {TeeKeyManager} from "src/attestation/TeeKeyManager.sol";

/// @dev RISC Zero Verifier Router for Groth16 proofs on Base Sepolia
/// See: https://dev.risczero.com/api/blockchain-integration/contracts/verifier
address constant RISC_ZERO_VERIFIER_ROUTER_BASE_SEPOLIA = 0x0b144E07A0826182B6b59788c34b32Bfa86Fb711;

/// @dev Default trusted JWK hashes for GCP Confidential Computing
/// These are keccak256 hashes of Google's attestation signing keys from:
/// https://confidentialcomputing.googleapis.com/v1alpha1/jwks
/// Keys may rotate, so additional hashes can be added via TRUSTED_JWK_HASHES env var.
string constant DEFAULT_TRUSTED_JWK_HASHES = "0xc4339aa224c54c5dcad4bf4d0183fd5a7d4eb346b3064b0c3ea938c415b19b5f";

/**
 * @title DeployRiscZeroAttestationVerifier
 * @notice Deployment script for TEE attestation verification contracts using RISC Zero
 * @dev Deploys RiscZeroAttestationVerifier and TeeKeyManager for GCP Confidential Space.
 *      After running this script, you must call keyManager.setBridge(bridgeAddress)
 *      once the Bridge is deployed.
 *
 *      Environment variables:
 *      - RISC_ZERO_VERIFIER_CONTRACT_ADDRESS: Optional, defaults to Base Sepolia router
 *      - RISC_ZERO_IMAGE_ID: Required, RISC Zero program image ID
 *      - ALLOWED_IMAGE_DIGEST_HASHES: Required, comma-separated list of allowed image digest hashes
 *      - EXPIRATION_TOLERANCE: Required, grace period in seconds
 *      - TRUSTED_JWK_HASHES: Optional, comma-separated list of JWK hashes
 *      - TRUSTED_IMAGE_SIGNERS: Optional, comma-separated list of trusted image signer addresses
 */
contract DeployRiscZeroAttestationVerifier is Script {
    function run() external returns (RiscZeroAttestationVerifier, TeeKeyManager) {
        address verifier = vm.envOr("RISC_ZERO_VERIFIER_CONTRACT_ADDRESS", RISC_ZERO_VERIFIER_ROUTER_BASE_SEPOLIA);
        bytes32 imageId = vm.envBytes32("RISC_ZERO_IMAGE_ID");
        string memory allowedImageDigestHashesStr = vm.envString("ALLOWED_IMAGE_DIGEST_HASHES");
        uint64 expirationTolerance = uint64(vm.envUint("EXPIRATION_TOLERANCE"));
        string memory trustedJwkHashesStr = vm.envOr("TRUSTED_JWK_HASHES", DEFAULT_TRUSTED_JWK_HASHES);
        string memory trustedImageSignersStr = vm.envOr("TRUSTED_IMAGE_SIGNERS", string(""));

        console.log("========================================");
        console.log("Deploying RISC Zero TEE Attestation Verifier");
        console.log("========================================");
        console.log("RISC Zero Verifier:", verifier);
        console.log("Image ID:", vm.toString(imageId));
        console.log("Allowed Image Digest Hashes:", allowedImageDigestHashesStr);
        console.log("Expiration Tolerance:", expirationTolerance);
        console.log("========================================");

        vm.startBroadcast();

        RiscZeroAttestationVerifier attestationVerifier =
            new RiscZeroAttestationVerifier(verifier, imageId, expirationTolerance);

        TeeKeyManager keyManager = new TeeKeyManager(attestationVerifier);

        // Add allowed image digest hashes (required)
        if (bytes(allowedImageDigestHashesStr).length > 0) {
            bytes32[] memory digestHashes = _parseBytes32Array(allowedImageDigestHashesStr);
            for (uint256 i = 0; i < digestHashes.length; i++) {
                attestationVerifier.addAllowedImageDigestHash(digestHashes[i]);
                console.log("Added allowed image digest hash:", vm.toString(digestHashes[i]));
            }
        }

        // Add trusted JWK hashes if provided
        if (bytes(trustedJwkHashesStr).length > 0) {
            bytes32[] memory jwkHashes = _parseBytes32Array(trustedJwkHashesStr);
            for (uint256 i = 0; i < jwkHashes.length; i++) {
                attestationVerifier.addTrustedJwkHash(jwkHashes[i]);
                console.log("Added trusted JWK hash:", vm.toString(jwkHashes[i]));
            }
        }

        // Add trusted image signers if provided
        if (bytes(trustedImageSignersStr).length > 0) {
            address[] memory signers = _parseAddresses(trustedImageSignersStr);
            for (uint256 i = 0; i < signers.length; i++) {
                attestationVerifier.addTrustedImageSigner(signers[i]);
                console.log("Added trusted image signer:", signers[i]);
            }
        }

        vm.stopBroadcast();

        console.log("========================================");
        console.log("Deployment Successful!");
        console.log("========================================");
        console.log("RiscZeroAttestationVerifier:", address(attestationVerifier));
        console.log("TeeKeyManager:", address(keyManager));
        console.log("========================================");
        console.log("");
        console.log("IMPORTANT: Complete setup after Bridge deployment:");
        console.log("1. Deploy Bridge with this TeeKeyManager address");
        console.log("2. Set Bridge on TeeKeyManager:");
        console.log("   keyManager.setBridge(<bridge_address>)");
        console.log("");
        console.log("3. Add trusted JWK hashes (if not already added):");
        console.log("   attestationVerifier.addTrustedJwkHash(<jwk_hash>)");
        console.log("");
        console.log("4. Add trusted image signers (if not already added):");
        console.log("   attestationVerifier.addTrustedImageSigner(<signer_address>)");
        console.log("");
        console.log("5. Register TEE keys through Bridge (0 = Sequencer, 1 = Validator):");
        console.log("   bridge.registerKey(0, <publicValues>, <proofBytes>)");
        console.log("========================================");

        return (attestationVerifier, keyManager);
    }

    /// @dev Parse comma-separated hex strings into bytes32 array
    function _parseBytes32Array(string memory input) internal pure returns (bytes32[] memory) {
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

    /// @dev Parse comma-separated addresses into address array
    function _parseAddresses(string memory input) internal pure returns (address[] memory) {
        // Count commas to determine array size
        bytes memory inputBytes = bytes(input);
        uint256 count = 1;
        for (uint256 i = 0; i < inputBytes.length; i++) {
            if (inputBytes[i] == ",") {
                count++;
            }
        }

        address[] memory result = new address[](count);
        uint256 start = 0;
        uint256 idx = 0;

        for (uint256 i = 0; i <= inputBytes.length; i++) {
            if (i == inputBytes.length || inputBytes[i] == ",") {
                // Extract substring
                bytes memory addrBytes = new bytes(i - start);
                for (uint256 j = start; j < i; j++) {
                    addrBytes[j - start] = inputBytes[j];
                }
                result[idx] = _hexToAddress(string(addrBytes));
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

    /// @dev Convert hex string to address (with or without 0x prefix)
    function _hexToAddress(string memory s) internal pure returns (address) {
        bytes memory ss = bytes(s);
        uint256 start = 0;

        // Skip 0x prefix if present
        if (ss.length >= 2 && ss[0] == "0" && (ss[1] == "x" || ss[1] == "X")) {
            start = 2;
        }

        require(ss.length - start == 40, "Invalid address hex string length");

        uint160 result = 0;
        for (uint256 i = 0; i < 20; i++) {
            uint8 high = _hexCharToUint8(ss[start + i * 2]);
            uint8 low = _hexCharToUint8(ss[start + i * 2 + 1]);
            result = uint160(uint256(result) | (uint256(high * 16 + low) << (8 * (19 - i))));
        }

        return address(result);
    }

    /// @dev Convert hex character to uint8
    function _hexCharToUint8(bytes1 c) internal pure returns (uint8) {
        if (c >= "0" && c <= "9") return uint8(c) - uint8(bytes1("0"));
        if (c >= "a" && c <= "f") return uint8(c) - uint8(bytes1("a")) + 10;
        if (c >= "A" && c <= "F") return uint8(c) - uint8(bytes1("A")) + 10;
        revert("Invalid hex character");
    }
}
