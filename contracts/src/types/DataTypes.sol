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
    ProcessingStage stage;
    bytes payload;
    uint256 createdAt;
}

struct SequencerSignature {
    bytes signature;
    uint256 submittedAt;
}
