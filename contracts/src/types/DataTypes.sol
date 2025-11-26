// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

enum ProcessingStage {
    NotStarted,
    PreExecution,
    Executing,
    PostExecution,
    Completed,
    Rejected
}

struct MessageState {
    bytes32 messageId;
    address targetAddress;
    ProcessingStage stage;
    bytes payload;
    uint256 createdAt;
    uint256 nativeTokenAmount;
}

struct SequencerSignature {
    bytes signature;
    uint256 submittedAt;
}
