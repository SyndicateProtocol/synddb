//! Data Availability publishing with pluggable backends
//!
//! This module provides a trait-based abstraction for publishing signed messages
//! to various storage/DA backends.
//!
//! # Transport Layers
//!
//! The `transport` module provides the CBOR-based transport abstraction:
//! - [`transport::TransportPublisher`] - Trait for CBOR batch storage
//! - [`transport_local::LocalTransport`] - Local storage with optional `SQLite` persistence
//! - [`transport_gcs::GcsTransport`] - GCS implementation (requires `gcs` feature)
//!
//! # Testing
//!
//! - [`mock::MockPublisher`] - In-memory mock for testing

pub mod mock;
pub mod traits;
pub mod transport;
pub mod transport_local;

// Update these with feature flags once implemented;
pub mod arweave;
pub mod celestia;
pub mod eigenda;
pub mod ipfs;

#[cfg(feature = "gcs")]
pub mod gcs;

#[cfg(feature = "gcs")]
pub mod transport_gcs;

pub mod transport_arweave;
