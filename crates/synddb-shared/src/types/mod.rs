//! Shared types for `SyndDB` wire format
//!
//! This module contains the core types used for communication between
//! `synddb-sequencer`, `synddb-validator`, and `synddb-client`.

mod message;
mod payloads;
pub mod serde_helpers;

// Re-export message types
pub use message::{MessageType, SequenceReceipt, SignedMessage};

// Re-export payload types
pub use payloads::{
    ChangesetBatchRequest, ChangesetData, SnapshotData, SnapshotRequest, WithdrawalRequest,
};

// Re-export serde helpers for convenience
pub use serde_helpers::base64_serde;
