// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {Test} from "forge-std/Test.sol";
import {Bridge} from "src/Bridge.sol";
import {MessageHashUtils} from "@openzeppelin/contracts/utils/cryptography/MessageHashUtils.sol";

/**
 * @title UseCaseBaseTest
 * @notice Base contract for use case tests with validator signature functionality
 */
abstract contract UseCaseBaseTest is Test {
    // Validators stored in arrays
    uint256[] public validatorPrivateKeys;
    address[] public validators;

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
