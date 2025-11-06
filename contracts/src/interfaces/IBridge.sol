// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {SequencerSignature} from "src/types/DataTypes.sol";

interface IBridge {
    function initializeMessage(
        bytes32 messageId,
        bytes calldata payload,
        SequencerSignature calldata sequencerSignature
    ) external;

    function handleMessage(bytes32 messageId) external;

    function initializeAndHandleMessage(
        bytes32 messageId,
        bytes calldata payload,
        SequencerSignature calldata sequencerSignature
    ) external;

    function isMessageExecuted(bytes32 messageId) external view returns (bool);
}
