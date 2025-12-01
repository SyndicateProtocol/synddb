//! DA provider implementations
//!
//! # Implemented Fetchers
//!
//! - [`GcsFetcher`] - Google Cloud Storage (feature: `gcs`)
//! - [`MockFetcher`] - In-memory mock for testing
//!
//! # TODO: Additional Fetchers
//!
//! The sequencer supports publishing to multiple DA layers. To achieve feature parity,
//! the following fetchers should be implemented:
//!
//! - **Celestia** (`celestia` feature) - Fetch from Celestia DA layer
//! - **`EigenDA`** (`eigenda` feature) - Fetch from `EigenDA`
//! - **Arweave** (`arweave` feature) - Fetch from Arweave permanent storage
//! - **IPFS** (`ipfs` feature) - Fetch from IPFS/Filecoin

#[cfg(feature = "gcs")]
pub mod gcs;

pub mod mock;

// TODO: Implement additional DA fetchers
// #[cfg(feature = "celestia")]
// pub mod celestia;
//
// #[cfg(feature = "eigenda")]
// pub mod eigenda;
//
// #[cfg(feature = "arweave")]
// pub mod arweave;
//
// #[cfg(feature = "ipfs")]
// pub mod ipfs;

#[cfg(feature = "gcs")]
pub use gcs::GcsFetcher;
pub use mock::MockFetcher;
