//! Data Availability publishing with pluggable backends
//!
//! This module provides a trait-based abstraction for publishing signed messages
//! to various storage/DA backends. Transport selection happens at runtime via
//! configuration (e.g., `PUBLISHER_TYPE=gcs`).
//!
//! # Transport Layers
//!
//! - [`traits::TransportPublisher`] - Trait for CBOR batch storage
//! - [`gcs::GcsTransport`] - Google Cloud Storage implementation
//! - [`local::LocalTransport`] - Local `SQLite` storage (default, also used for testing)
//!
//! # Future Transports
//!
//! When implemented, additional transports will be available via runtime config:
//! - Arweave: Permanent storage via ANS-104 bundles
//! - Celestia DA layer
//! - `EigenDA` layer

pub mod gcs;
pub mod local;
pub mod traits;

// TODO: Implement additional DA layer transports (see PLAN_TODO.md)
pub mod arweave;
