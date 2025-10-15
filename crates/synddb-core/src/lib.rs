//! # SyndDB Core
//!
//! SyndDB Core is the foundational infrastructure for building high-performance
//! blockchain databases. It provides:
//!
//! - High-performance SQLite execution engine optimized for blockchain workloads
//! - State replication and synchronization between sequencer and replicas
//! - Extension system for building custom business logic
//! - Local write framework with ultra-low latency (<1ms)
//! - Blockchain integration for durability and decentralization
//!
//! ## Architecture
//!
//! SyndDB follows a Core + Extensions architecture:
//!
//! - **Core**: Provides database execution, state replication, and blockchain integration
//! - **Extensions**: Implement business logic, schemas, triggers, and bridge operations
//!
//! ## Example Usage
//!
//! ```rust,no_run
//! use synddb_core::{
//!     database::SqliteDatabase,
//!     config::SyndDBConfig,
//!     extensions::ExtensionRegistry,
//!     writes::LocalWriteProcessor,
//! };
//! use std::sync::Arc;
//!
//! # async fn example() -> synddb_core::types::Result<()> {
//! // Load configuration
//! let config = SyndDBConfig::test_config();
//!
//! // Initialize database
//! let database = Arc::new(SqliteDatabase::new(
//!     &config.database.path,
//!     config.database.pool_size,
//! )?);
//!
//! // Create extension registry
//! let mut registry = ExtensionRegistry::new();
//! // Register your extensions here...
//!
//! // Initialize extensions
//! registry.initialize(database.as_ref()).await?;
//!
//! // Create write processor
//! let processor = LocalWriteProcessor::new(
//!     Arc::new(registry),
//!     database.clone(),
//! );
//!
//! // Process writes
//! // let receipt = processor.execute_local_write(write).await?;
//! # Ok(())
//! # }
//! ```
//!
//! ## Extension Development
//!
//! To build on SyndDB Core, implement the extension traits:
//!
//! - [`SchemaExtension`](extensions::SchemaExtension) - Define database schemas
//! - [`LocalWriteExtension`](extensions::LocalWriteExtension) - Define write operations
//! - [`TriggerExtension`](extensions::TriggerExtension) - Define automated business logic
//! - [`BridgeExtension`](extensions::BridgeExtension) - Define bridge operations
//! - [`QueryExtension`](extensions::QueryExtension) - Define custom queries
//!
//! See the [SyndDB Extensions Plan](../../PLAN_EXTENSIONS.md) for detailed guides.

// Public modules
pub mod config;
pub mod database;
pub mod extensions;
pub mod metrics;
pub mod types;
pub mod writes;

// Re-export commonly used types for convenience
pub use types::{
    DatabaseDiff, DatabaseSnapshot, DatabaseTransaction, Error, LocalWrite, LocalWriteReceipt,
    LocalWriteStatus, QueryResult, Result, SqlOperation, SqlValue,
};

pub use database::{SqliteDatabase, SyndDatabase};
pub use extensions::{
    BridgeExtension, ExtensionRegistry, LocalWriteExtension, QueryExtension, SchemaExtension,
    TriggerEvent, TriggerExtension,
};
pub use writes::{LocalWriteBuilder, LocalWriteProcessor};

/// Version information
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Get version string
pub fn version() -> &'static str {
    VERSION
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version() {
        let v = version();
        assert!(!v.is_empty());
    }
}
