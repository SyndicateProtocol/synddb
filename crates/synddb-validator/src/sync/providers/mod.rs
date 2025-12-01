//! DA provider implementations

#[cfg(feature = "gcs")]
pub mod gcs;

pub mod mock;

#[cfg(feature = "gcs")]
pub use gcs::GcsFetcher;
pub use mock::MockFetcher;
