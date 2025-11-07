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
    // Validators
    uint256 public validator1PrivateKey;
    uint256 public validator2PrivateKey;
    uint256 public validator3PrivateKey;
    address public validator1;
    address public validator2;
    address public validator3;
    address[] public validators;

    /// @notice Setup validators with known private keys
    function setupValidators(Bridge bridge) internal {
        validator1PrivateKey = 0x1;
        validator2PrivateKey = 0x2;
        validator3PrivateKey = 0x3;
        validator1 = vm.addr(validator1PrivateKey);
        validator2 = vm.addr(validator2PrivateKey);
        validator3 = vm.addr(validator3PrivateKey);

        validators.push(validator1);
        validators.push(validator2);
        validators.push(validator3);

        bridge.grantRole(bridge.VALIDATOR_ROLE(), validator1);
        bridge.grantRole(bridge.VALIDATOR_ROLE(), validator2);
        bridge.grantRole(bridge.VALIDATOR_ROLE(), validator3);
    }

    /// @notice Helper function to sign message with validator private key
    function signMessage(bytes32 messageId, uint256 validatorPrivateKey) internal pure returns (bytes memory) {
        bytes32 ethSignedMessageHash = MessageHashUtils.toEthSignedMessageHash(messageId);
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(validatorPrivateKey, ethSignedMessageHash);
        return abi.encodePacked(r, s, v);
    }

    /// @notice Helper function to submit validator signatures (2 of 3 threshold)
    function submitValidatorSignatures(Bridge bridge, bytes32 messageId) internal {
        bytes memory sig1 = signMessage(messageId, validator1PrivateKey);
        bytes memory sig2 = signMessage(messageId, validator2PrivateKey);
        bridge.signMessageWithSignature(messageId, sig1);
        bridge.signMessageWithSignature(messageId, sig2);
    }
}
