//! Data Availability publishing with pluggable backends
//!
//! This module provides a trait-based abstraction for publishing signed messages
//! to various storage/DA backends.
//!
//! # Transport Layers
//!
//! The `transport` module provides the CBOR-based transport abstraction:
//! - [`traits::TransportPublisher`] - Trait for CBOR batch storage
//! - [`gcs::GcsTransport`] - GCS implementation (requires `gcs` feature)
//!
//! # Testing
//!
//! - [`local::LocalTransport`] - Local storage with optional `SQLite` persistence,
//!   otherwise defaults to in-memory persistence

pub mod local;
pub mod traits;

#[cfg(feature = "gcs")]
pub mod gcs;

// TODO: Implement additional DA layer transports (see PLAN_TODO.md)
// - arweave: Permanent storage via ANS-104 bundles (detailed plan in arweave.rs)
// - celestia: Celestia DA layer
// - eigenda: EigenDA layer
// - ipfs: IPFS storage
pub mod arweave;
