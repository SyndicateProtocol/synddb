//! Storage abstraction for `SyndDB`
//!
//! This crate provides a unified interface for object storage operations,
//! currently supporting Google Cloud Storage with built-in emulator support
//! for testing with fake-gcs-server.
//!
//! # Example
//!
//! ```ignore
//! use synddb_storage::{StorageClient, StorageConfig};
//!
//! // Production
//! let config = StorageConfig::gcs("my-bucket", "prefix");
//! let client = StorageClient::new(config).await?;
//!
//! // With emulator
//! let config = StorageConfig::gcs("test-bucket", "prefix")
//!     .with_emulator("http://localhost:4443");
//! let client = StorageClient::new(config).await?;
//!
//! // Operations
//! client.write("path/to/object", data).await?;
//! let data = client.read("path/to/object").await?;
//! let objects = client.list("path/prefix/").await?;
//! ```

mod client;
mod config;
mod error;

pub use client::{ObjectInfo, StorageClient};
pub use config::StorageConfig;
pub use error::StorageError;

// Re-export GcsConfig for backwards compatibility
pub use synddb_shared::gcs::GcsConfig;
