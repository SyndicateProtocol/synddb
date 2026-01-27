//! Shared GCS configuration for sequencer and validator
//!
//! This module provides a common configuration struct for Google Cloud Storage
//! that can be used by both the sequencer (for publishing) and validator (for fetching).

use serde::{Deserialize, Serialize};

/// Configuration for Google Cloud Storage access
///
/// Used by both the sequencer's `GcsTransport` and validator's `GcsFetcher`.
///
/// # Examples
///
/// ```
/// use synddb_shared::gcs::GcsConfig;
///
/// // Production configuration
/// let config = GcsConfig::new("my-bucket")
///     .with_prefix("sequencer/v1");
///
/// // Local testing with emulator
/// let config = GcsConfig::new("test-bucket")
///     .with_emulator_host("http://localhost:4443");
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GcsConfig {
    /// GCS bucket name
    pub bucket: String,
    /// Path prefix within the bucket (default: "sequencer")
    pub prefix: String,
    /// GCS emulator host URL for local testing
    ///
    /// When set, the client uses anonymous authentication and connects to
    /// the specified emulator (e.g., `fake-gcs-server`) instead of real GCS.
    ///
    /// Example: `http://localhost:4443` or `http://fake-gcs:4443` in Docker.
    pub emulator_host: Option<String>,
}

impl GcsConfig {
    /// Create a new GCS config with the specified bucket
    ///
    /// Uses "sequencer" as the default prefix.
    pub fn new(bucket: impl Into<String>) -> Self {
        Self {
            bucket: bucket.into(),
            prefix: "sequencer".to_string(),
            emulator_host: None,
        }
    }

    /// Set the path prefix within the bucket
    #[must_use]
    pub fn with_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.prefix = prefix.into();
        self
    }

    /// Set the emulator host URL for local testing
    ///
    /// Pass an empty string to disable emulator mode.
    #[must_use]
    pub fn with_emulator_host(mut self, host: impl Into<String>) -> Self {
        let host = host.into();
        self.emulator_host = if host.is_empty() { None } else { Some(host) };
        self
    }

    /// Check if this config is using an emulator
    pub const fn is_emulator(&self) -> bool {
        self.emulator_host.is_some()
    }

    /// Get the full path for a batch file
    ///
    /// Returns a path like `{prefix}/batches/{filename}`
    pub fn batch_path(&self, filename: &str) -> String {
        format!("{}/batches/{}", self.prefix, filename)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_config() {
        let config = GcsConfig::new("my-bucket");
        assert_eq!(config.bucket, "my-bucket");
        assert_eq!(config.prefix, "sequencer");
        assert!(config.emulator_host.is_none());
        assert!(!config.is_emulator());
    }

    #[test]
    fn test_with_prefix() {
        let config = GcsConfig::new("bucket").with_prefix("custom/path");
        assert_eq!(config.prefix, "custom/path");
    }

    #[test]
    fn test_with_emulator() {
        let config = GcsConfig::new("bucket").with_emulator_host("http://localhost:4443");
        assert_eq!(
            config.emulator_host,
            Some("http://localhost:4443".to_string())
        );
        assert!(config.is_emulator());
    }

    #[test]
    fn test_empty_emulator_host() {
        let config = GcsConfig::new("bucket").with_emulator_host("");
        assert!(config.emulator_host.is_none());
        assert!(!config.is_emulator());
    }

    #[test]
    fn test_batch_path() {
        let config = GcsConfig::new("bucket").with_prefix("sequencer");
        assert_eq!(
            config.batch_path("000000000001_000000000050.cbor.zst"),
            "sequencer/batches/000000000001_000000000050.cbor.zst"
        );
    }
}
