//! DA provider implementations
//!
//! # Implemented Fetchers
//!
//! - [`GcsFetcher`] - Google Cloud Storage
//! - [`MockFetcher`] - In-memory mock for testing

pub mod gcs;
pub mod mock;

pub use gcs::GcsFetcher;
pub use mock::MockFetcher;
