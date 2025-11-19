//! Changeset and schema change types
//!
//! These types define the format for changesets and schema changes
//! that are received from client libraries via HTTP. The changesets
//! use SQLite Session Extension format (deterministic, auditable).

use serde::{Deserialize, Serialize};
use std::time::SystemTime;

/// A changeset received from a client library
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

/// Schema change received from a client library
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
