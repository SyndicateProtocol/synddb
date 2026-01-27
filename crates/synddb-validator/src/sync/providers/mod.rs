//! Storage provider implementations
//!
//! # Implemented Fetchers
//!
//! - [`http::HttpFetcher`] - HTTP fetcher for sequencer's local storage API
//! - [`gcs::GcsFetcher`] - Google Cloud Storage
//! - [`mock::MockFetcher`] - In-memory mock for testing

pub mod gcs;
pub mod http;
pub mod mock;
