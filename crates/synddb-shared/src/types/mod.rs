//! Shared types for `SyndDB` wire format
//!
//! This module contains the core types used for communication between
//! `synddb-sequencer`, `synddb-validator`, and `synddb-client`.
//!
//! Types are organized in submodules:
//! - `message`: Core message types (`SignedMessage`, `MessageType`, `SequenceReceipt`)
//! - `payloads`: Request/response payload types
//! - `serde_helpers`: Serialization utilities (e.g., base64)

pub mod message;
pub mod payloads;
pub mod serde_helpers;

pub use message::{MessageType, SequenceReceipt, SignedMessage};
pub use payloads::{
    ChangesetBatchRequest, ChangesetData, SnapshotData, SnapshotRequest, WithdrawalRequest,
};
