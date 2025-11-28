// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {Test} from "forge-std/Test.sol";
import {Bridge} from "src/Bridge.sol";
import {ValidatorSignatureThresholdModule} from "src/modules/ValidatorSignatureThresholdModule.sol";
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
        bridge = new Bridge(admin, address(weth));
        bridge.grantRole(bridge.SEQUENCER_ROLE(), sequencer);
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
        validatorModule = new ValidatorSignatureThresholdModule(address(bridge), validators, threshold);
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
    /// @param bridge The bridge contract to grant validator roles
    /// @param validatorCount The number of validators to create (defaults to 3)
    function setupValidators(Bridge bridge, uint256 validatorCount) internal {
        for (uint256 i = 1; i <= validatorCount; i++) {
            uint256 privateKey = i;
            address validatorAddr = vm.addr(privateKey);

            validatorPrivateKeys.push(privateKey);
            validators.push(validatorAddr);

            bridge.grantRole(bridge.VALIDATOR_ROLE(), validatorAddr);
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
