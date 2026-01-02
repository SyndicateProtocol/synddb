//! Prelude module for convenient imports
//!
//! This module re-exports the most commonly used types from `synddb-shared`.
//! Use it to reduce boilerplate in files that use multiple shared types:
//!
//! ```rust
//! use synddb_shared::prelude::*;
//! ```
//!
//! # Included Types
//!
//! ## Key Management
//! - [`EvmKeyManager`] - Secure secp256k1 key generation for EVM signing
//!
//! ## Message Types
//! - [`SignedMessage`] - A signed message from the sequencer
//! - [`SignedBatch`] - A batch of signed messages
//! - [`MessageType`] - Type of message (changeset, snapshot, etc.)
//!
//! ## Batch Utilities
//! - [`BatchInfo`] - Metadata about a batch in storage
//!
//! ## HTTP Payloads
//! - [`ChangesetData`] - Changeset data from client
//! - [`ChangesetBatchRequest`] - Batch request from client
//! - [`SnapshotData`] - Snapshot data from client
//! - [`SnapshotRequest`] - Snapshot request from client
//! - [`BatchListItem`] - Batch list response item

// Key management
pub use crate::keys::EvmKeyManager;

// Message types
pub use crate::types::message::{MessageType, SignedBatch, SignedMessage};

// Batch utilities
pub use crate::types::batch::BatchInfo;

// HTTP payloads
pub use crate::types::payloads::{
    BatchListItem, ChangesetBatchRequest, ChangesetData, SnapshotData, SnapshotRequest,
};
