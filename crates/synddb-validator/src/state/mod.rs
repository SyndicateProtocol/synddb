//! State persistence module
//!
//! Tracks sync progress and validator state for crash recovery.

mod store;

pub use store::StateStore;
