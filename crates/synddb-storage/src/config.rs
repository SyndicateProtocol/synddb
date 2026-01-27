//! Storage configuration

use synddb_shared::gcs::GcsConfig;

/// Configuration for storage backends
#[derive(Debug, Clone)]
pub enum StorageConfig {
    /// Google Cloud Storage
    Gcs(GcsConfig),
    // Future: S3, Azure, Local, etc.
}

impl StorageConfig {
    /// Create a GCS configuration
    pub fn gcs(bucket: impl Into<String>, prefix: impl Into<String>) -> Self {
        Self::Gcs(GcsConfig::new(bucket).with_prefix(prefix))
    }

    /// Set the emulator host (for GCS)
    #[must_use]
    pub fn with_emulator(self, host: impl Into<String>) -> Self {
        match self {
            Self::Gcs(config) => Self::Gcs(config.with_emulator_host(host)),
        }
    }

    /// Check if using an emulator
    pub const fn is_emulator(&self) -> bool {
        match self {
            Self::Gcs(config) => config.is_emulator(),
        }
    }

    /// Get the bucket name (for GCS)
    pub fn bucket(&self) -> &str {
        match self {
            Self::Gcs(config) => &config.bucket,
        }
    }

    /// Get the prefix
    pub fn prefix(&self) -> &str {
        match self {
            Self::Gcs(config) => &config.prefix,
        }
    }

    /// Get the emulator host if configured
    pub fn emulator_host(&self) -> Option<&str> {
        match self {
            Self::Gcs(config) => config.emulator_host.as_deref(),
        }
    }
}

impl From<GcsConfig> for StorageConfig {
    fn from(config: GcsConfig) -> Self {
        Self::Gcs(config)
    }
}
