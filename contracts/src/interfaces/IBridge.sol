// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {SequencerSignature} from "src/types/DataTypes.sol";

interface IBridge {
    function initializeMessage(
        bytes32 messageId,
        address targetAddress,
        bytes calldata payload,
        SequencerSignature calldata sequencerSignature,
        uint256 nativeTokenAmount
    ) external;

    function handleMessage(bytes32 messageId) external;

    function initializeAndHandleMessage(
        bytes32 messageId,
        address targetAddress,
        bytes calldata payload,
        SequencerSignature calldata sequencerSignature,
        bytes[] calldata validatorSignatures,
        uint256 nativeTokenAmount
    ) external;

    function isMessageCompleted(bytes32 messageId) external view returns (bool);

    function isMessageRejected(bytes32 messageId) external view returns (bool);

    function isMessageHandled(bytes32 messageId) external view returns (bool);

    function isMessageInitialized(bytes32 messageId) external view returns (bool);
}
