//! Storage provider implementations
//!
//! # Implemented Fetchers
//!
//! - [`HttpFetcher`] - HTTP fetcher for sequencer's local storage API
//! - [`GcsFetcher`] - Google Cloud Storage
//! - [`MockFetcher`] - In-memory mock for testing

pub mod gcs;
pub mod http;
pub mod mock;
