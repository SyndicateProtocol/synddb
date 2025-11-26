//! Data Availability publishing with pluggable backends
//!
//! This module provides a trait-based abstraction for publishing signed messages
//! to various storage/DA backends. The first implementation is Google Cloud Storage.

pub mod mock;
pub mod traits;

// Update these with feature flags once implemented;
pub mod arweave;
pub mod celestia;
pub mod eigenda;
pub mod ipfs;

#[cfg(feature = "gcs")]
pub mod gcs;
