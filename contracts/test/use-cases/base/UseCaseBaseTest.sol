// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {Test} from "forge-std/Test.sol";
import {Bridge} from "src/Bridge.sol";
import {ValidatorSignatureThresholdModule} from "src/modules/ValidatorSignatureThresholdModule.sol";
import {TeeKeyManager} from "src/attestation/TeeKeyManager.sol";
import {MockAttestationVerifier} from "src/attestation/MockAttestationVerifier.sol";
import {SequencerSignature} from "src/types/DataTypes.sol";
import {WETH9} from "../mocks/WETH9.sol";
import {MessageHashUtils} from "@openzeppelin/contracts/utils/cryptography/MessageHashUtils.sol";

/**
 * @title UseCaseBaseTest
 * @notice Base contract for use case tests with validator signature functionality and shared setup helpers
 */
abstract contract UseCaseBaseTest is Test {
    // Validators stored in arrays
    uint256[] public validatorPrivateKeys;
    address[] public validators;

    // Sequencer key for signing messages
    uint256 public sequencerPrivateKey = 0xA11CE;

    // TEE infrastructure
    TeeKeyManager public teeKeyManager;
    MockAttestationVerifier public attestationVerifier;

    /*//////////////////////////////////////////////////////////////
                            SHARED SETUP HELPERS
    //////////////////////////////////////////////////////////////*/

    /// @notice Create a basic Bridge with WETH and grant sequencer role
    /// @param admin The admin address (typically address(this))
    /// @param sequencer The sequencer address
    /// @return bridge The deployed bridge
    /// @return weth The deployed WETH9 contract
    function createBridgeWithWETH(address admin, address sequencer) internal returns (Bridge bridge, WETH9 weth) {
        weth = new WETH9();

        // Deploy attestation infrastructure
        attestationVerifier = new MockAttestationVerifier();
        teeKeyManager = new TeeKeyManager(attestationVerifier);

        // Deploy bridge first
        bridge = new Bridge(admin, address(weth), address(teeKeyManager));

        // Set bridge on TeeKeyManager
        teeKeyManager.setBridge(address(bridge));

        // Register sequencer as a valid TEE key through bridge
        bytes memory publicValues = abi.encode(sequencer);
        bridge.registerSequencerKey(publicValues, "");

        // Grant message initializer permission
        bridge.setMessageInitializer(sequencer, true);
    }

    /// @notice Create a sequencer signature for a message
    function createSequencerSignature(
        bytes32 messageId,
        address targetAddress,
        bytes memory payload,
        uint256 nativeTokenAmount
    ) internal view returns (SequencerSignature memory) {
        bytes32 messageHash = keccak256(
            abi.encodePacked(messageId, targetAddress, keccak256(payload), nativeTokenAmount)
        );
        bytes32 ethSignedHash = MessageHashUtils.toEthSignedMessageHash(messageHash);
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(sequencerPrivateKey, ethSignedHash);
        return SequencerSignature({signature: abi.encodePacked(r, s, v), submittedAt: block.timestamp});
    }

    /// @notice Setup bridge with validators and validator module
    /// @param bridge The bridge contract
    /// @param threshold The signature threshold (defaults to 2)
    /// @return validatorModule The deployed validator module
    function setupBridgeWithValidators(Bridge bridge, uint256 threshold)
        internal
        returns (ValidatorSignatureThresholdModule validatorModule)
    {
        setupValidators(bridge);
        validatorModule = new ValidatorSignatureThresholdModule(address(bridge), address(teeKeyManager), threshold);
        bridge.addPreModule(address(validatorModule));
    }

    /// @notice Setup bridge with validators using default threshold of 2
    function setupBridgeWithValidators(Bridge bridge)
        internal
        returns (ValidatorSignatureThresholdModule validatorModule)
    {
        return setupBridgeWithValidators(bridge, 2);
    }

    /*//////////////////////////////////////////////////////////////
                        VALIDATOR SETUP & SIGNING
    //////////////////////////////////////////////////////////////*/

    /// @notice Setup validators with known private keys
    /// @param bridge The bridge contract to register validator keys
    /// @param validatorCount The number of validators to create (defaults to 3)
    function setupValidators(Bridge bridge, uint256 validatorCount) internal {
        for (uint256 i = 1; i <= validatorCount; i++) {
            uint256 privateKey = i;
            address validatorAddr = vm.addr(privateKey);

            validatorPrivateKeys.push(privateKey);
            validators.push(validatorAddr);

            // Register validator key through bridge
            bytes memory publicValues = abi.encode(validatorAddr);
            bridge.registerValidatorKey(publicValues, "");
        }
    }

    /// @notice Setup validators with default count of 3
    function setupValidators(Bridge bridge) internal {
        setupValidators(bridge, 3);
    }

    /// @notice Helper function to sign message with validator private key
    function signMessage(bytes32 messageId, uint256 validatorPrivateKey) internal pure returns (bytes memory) {
        bytes32 ethSignedMessageHash = MessageHashUtils.toEthSignedMessageHash(messageId);
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(validatorPrivateKey, ethSignedMessageHash);
        return abi.encodePacked(r, s, v);
    }

    /// @notice Helper function to submit validator signatures (2 of 3 threshold by default)
    /// @param bridge The bridge contract
    /// @param messageId The message ID to sign
    /// @param signatureCount The number of signatures to submit (defaults to 2)
    function submitValidatorSignatures(Bridge bridge, bytes32 messageId, uint256 signatureCount) internal {
        require(signatureCount <= validatorPrivateKeys.length, "Not enough validators");

        for (uint256 i = 0; i < signatureCount; i++) {
            bytes memory sig = signMessage(messageId, validatorPrivateKeys[i]);
            bridge.signMessageWithSignature(messageId, sig);
        }
    }

    /// @notice Helper function to submit default 2 validator signatures
    function submitValidatorSignatures(Bridge bridge, bytes32 messageId) internal {
        submitValidatorSignatures(bridge, messageId, 2);
    }
}
