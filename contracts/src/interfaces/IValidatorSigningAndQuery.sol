// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

interface IValidatorSigningAndQuery {
    event MessageSigned(bytes32 indexed messageId, address indexed validator);

    error ValidatorNotAuthorized();

    function signMessage(bytes32 messageId) external;

    function signMessageWithSignature(bytes32 messageId, bytes calldata signature) external;

    function getValidatorSignatureCount(bytes32 messageId, address[] calldata validators)
        external
        view
        returns (uint256);
}
