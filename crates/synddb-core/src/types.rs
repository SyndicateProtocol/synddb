//! Core type definitions for SyndDB
//!
//! This module contains the fundamental data structures used throughout SyndDB,
//! including database operations, state management, and replication types.

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Result type alias for SyndDB operations
pub type Result<T> = std::result::Result<T, Error>;

/// Core error types for SyndDB
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Compression error: {0}")]
    Compression(String),

    #[error("Configuration error: {0}")]
    Config(#[from] config::ConfigError),

    #[error("Extension error: {0}")]
    Extension(String),

    #[error("Invalid operation: {0}")]
    InvalidOperation(String),

    #[error("State error: {0}")]
    State(String),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

// ============================================================================
// Local Writes (Sequencer Execution)
// ============================================================================

/// A local write operation executed immediately in the sequencer's SQLite database.
/// These operations provide ultra-low latency (<1ms) since there's no distributed consensus.
/// The write is durable locally but not yet replicated to other nodes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalWrite {
    /// Type of write operation (defined by extensions)
    pub write_type: String,
    /// JSON request payload (validated and converted to SQL by extensions)
    pub request: serde_json::Value,
    /// Unix timestamp in milliseconds
    pub timestamp: u64,
    /// Monotonic nonce for ordering
    pub nonce: u64,
}

/// Receipt returned after executing a local write
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalWriteReceipt {
    /// Unique identifier for this write
    pub write_id: String,
    /// Status of local execution
    pub status: LocalWriteStatus,
    /// Latency of local execution
    pub latency: Duration,
    /// Estimated time until replication
    pub replication_eta: String,
}

/// Status of a local write operation
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LocalWriteStatus {
    /// Successfully committed to local database
    CommittedLocally,
    /// Failed during local execution
    Failed(String),
}

impl std::fmt::Display for LocalWriteStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LocalWriteStatus::CommittedLocally => write!(f, "Committed Locally"),
            LocalWriteStatus::Failed(reason) => write!(f, "Failed: {}", reason),
        }
    }
}

// ============================================================================
// Database State Replication
// ============================================================================

/// Complete database state at a specific version (like a backup).
/// Used by read replicas to bootstrap from a known state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseSnapshot {
    /// Version this snapshot represents
    pub version: u64,
    /// Complete SQLite database file (compressed)
    pub data: Vec<u8>,
    /// Size before compression
    pub uncompressed_size: usize,
    /// Size after compression
    pub compressed_size: usize,
    /// Checksum for integrity verification
    pub checksum: String,
    /// Unix timestamp in milliseconds
    pub timestamp: u64,
}

/// Incremental database changes between versions (like git diff).
/// Used by read replicas to sync incrementally.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseDiff {
    /// Starting version
    pub from_version: u64,
    /// Ending version
    pub to_version: u64,
    /// SQL statements to replay these changes
    pub statements: Vec<String>,
    /// Compressed SQL statements
    pub compressed: Vec<u8>,
    /// Size after compression
    pub compressed_size: usize,
    /// Compression ratio
    pub compression_ratio: f64,
    /// Checksum for integrity verification
    pub checksum: String,
    /// Unix timestamp in milliseconds
    pub timestamp: u64,
}

// ============================================================================
// SQL Operations & Results
// ============================================================================

/// A single SQL operation to execute
#[derive(Debug, Clone)]
pub struct SqlOperation {
    /// SQL statement
    pub sql: String,
    /// Parameters for the SQL statement
    pub params: Vec<SqlValue>,
}

/// SQL parameter value
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub enum SqlValue {
    #[default]
    Null,
    Integer(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

/// Result of executing a SQL operation
#[derive(Debug, Clone)]
pub struct ExecuteResult {
    /// Number of rows affected
    pub rows_affected: usize,
    /// Last inserted row ID (if applicable)
    pub last_insert_rowid: Option<i64>,
    /// Execution time
    pub duration: Duration,
}

/// Result of executing a batch of SQL operations
#[derive(Debug, Clone)]
pub struct BatchResult {
    /// Whether the batch succeeded
    pub success: bool,
    /// Results for each operation
    pub results: Vec<ExecuteResult>,
    /// Total duration
    pub duration: Duration,
}

/// Result of a query operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryResult {
    /// Column names
    pub columns: Vec<String>,
    /// Rows of data
    pub rows: Vec<Vec<SqlValue>>,
    /// Number of rows returned
    pub row_count: usize,
    /// Query execution time
    pub duration: Duration,
}

// ============================================================================
// Blockchain Publication
// ============================================================================

/// Receipt from submitting writes to the blockchain
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainSubmitReceipt {
    /// Transaction hash on the blockchain
    pub tx_hash: String,
    /// Block number where the transaction was included
    pub block_number: u64,
    /// Version range submitted
    pub from_version: u64,
    /// Version range submitted
    pub to_version: u64,
    /// Type of submission
    pub submission_type: ChainSubmissionType,
    /// Gas used
    pub gas_used: u64,
}

/// Type of blockchain submission
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChainSubmissionType {
    /// Direct diff storage on-chain
    DiffDirect,
    /// Diff pointer to off-chain storage
    DiffPointer(String),
    /// Direct snapshot storage on-chain
    SnapshotDirect,
    /// Snapshot pointer to off-chain storage
    SnapshotPointer(String),
}

impl std::fmt::Display for ChainSubmissionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChainSubmissionType::DiffDirect => write!(f, "Direct Diff"),
            ChainSubmissionType::DiffPointer(ptr) => write!(f, "Diff Pointer ({})", ptr),
            ChainSubmissionType::SnapshotDirect => write!(f, "Direct Snapshot"),
            ChainSubmissionType::SnapshotPointer(ptr) => write!(f, "Snapshot Pointer ({})", ptr),
        }
    }
}

// ============================================================================
// Database Transaction Handle
// ============================================================================

/// Handle to a database transaction
pub struct DatabaseTransaction {
    /// Transaction ID
    pub id: String,
    /// Version when transaction started
    pub version: u64,
}

// ============================================================================
// Performance Metrics
// ============================================================================

/// Performance statistics for the database
#[derive(Debug, Clone, Default)]
pub struct PerformanceStats {
    /// Total number of operations executed
    pub total_operations: u64,
    /// Average operation latency in microseconds
    pub avg_latency_us: f64,
    /// P50 latency in microseconds
    pub p50_latency_us: u64,
    /// P99 latency in microseconds
    pub p99_latency_us: u64,
    /// Operations per second
    pub ops_per_second: f64,
}

// ============================================================================
// Utility Functions
// ============================================================================

/// Generate a unique write ID
pub fn generate_write_id() -> String {
    use sha3::{Digest, Sha3_256};
    let mut hasher = Sha3_256::new();
    hasher.update(uuid::Uuid::new_v4().as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Calculate checksum for data
pub fn calculate_checksum(data: &[u8]) -> String {
    use sha3::{Digest, Sha3_256};
    let mut hasher = Sha3_256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

/// Get current Unix timestamp in milliseconds
///
/// # Panics
/// Panics if the system clock is set before the Unix epoch (January 1, 1970).
/// This should never happen on any modern system.
pub fn current_timestamp_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("System clock set before Unix epoch")
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_write_id() {
        let id1 = generate_write_id();
        let id2 = generate_write_id();
        assert_ne!(id1, id2);
        assert_eq!(id1.len(), 64); // SHA3-256 produces 64 hex characters
    }

    #[test]
    fn test_calculate_checksum() {
        let data = b"hello world";
        let checksum = calculate_checksum(data);
        assert_eq!(checksum.len(), 64);

        // Same data should produce same checksum
        let checksum2 = calculate_checksum(data);
        assert_eq!(checksum, checksum2);
    }
}
