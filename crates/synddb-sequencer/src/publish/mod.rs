//! Data Availability publishing with pluggable backends
//!
//! This module provides a trait-based abstraction for publishing signed messages
//! to various storage/DA backends. The first implementation is Google Cloud Storage.

mod mock;
mod traits;

#[cfg(feature = "gcs")]
pub mod gcs;

// // Re-export main types
pub use mock::MockPublisher;
pub use traits::{DAPublisher, PublishError, PublishResult};

//
// // Re-export for tests
// #[cfg(test)]
// pub mod tests {
//     pub use super::mock::MockPublisher;
// }
