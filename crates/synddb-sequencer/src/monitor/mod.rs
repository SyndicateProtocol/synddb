//! Monitoring types module
//!
//! Defines data structures for changesets and schema changes received
//! from synddb-client libraries.

use serde::{Deserialize, Serialize};
use std::time::SystemTime;

/// A captured changeset from the database
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Changeset {
    /// Raw changeset data from SQLite Session Extension
    pub data: Vec<u8>,
    /// Sequence number for ordering
    pub sequence: u64,
    /// Timestamp when captured
    pub timestamp: SystemTime,
    /// Size in bytes
    pub size: usize,
}

/// Schema change captured from DDL operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaChange {
    /// Previous schema version (from PRAGMA user_version)
    pub old_version: i32,
    /// New schema version
    pub new_version: i32,
    /// DDL statements that changed the schema
    pub ddl_statements: Vec<String>,
    /// Timestamp when captured
    pub timestamp: SystemTime,
}
