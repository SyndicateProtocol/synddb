//! Core database abstraction layer for SyndDB
//!
//! This module provides the main database trait and implementation using SQLite
//! with performance optimizations for high-throughput blockchain workloads.

use crate::metrics::MetricsCollector;
use crate::prepared_statements::PreparedStatementCache;
use crate::types::*;
use async_trait::async_trait;
use r2d2::{Pool, PooledConnection};
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{Connection, OptionalExtension, ToSql};
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;
use tracing::{debug, info};

// ============================================================================
// Core Database Trait
// ============================================================================

/// Main database trait for SyndDB
///
/// This trait defines the core database operations including:
/// - Local write execution (sequencer only)
/// - State snapshot/diff generation (sequencer only)
/// - State snapshot/diff application (replica only)
#[async_trait]
pub trait SyndDatabase: Send + Sync {
    /// Begin a database transaction
    async fn begin_database_transaction(&self) -> Result<DatabaseTransaction>;

    /// Execute a SQL statement with parameters (simple string version)
    async fn execute(&self, sql: &str, params: Vec<SqlValue>) -> Result<ExecuteResult>;

    /// Execute a batch of SQL operations atomically
    async fn execute_batch(&self, operations: Vec<SqlOperation>) -> Result<BatchResult>;

    /// Query the database
    async fn query(&self, sql: &str, params: Vec<SqlValue>) -> Result<QueryResult>;

    // ========================================================================
    // SEQUENCER METHODS - Create replication data from local state
    // ========================================================================

    /// Generate a full database snapshot at the current version
    ///
    /// Creates a complete database backup (like `pg_dump` or `mysqldump`)
    /// for replica bootstrapping. The snapshot contains the entire SQLite
    /// database file, compressed for efficient storage.
    async fn generate_snapshot(&self) -> Result<DatabaseSnapshot>;

    /// Generate incremental changes between two versions
    ///
    /// Creates a diff containing just the changes (like git diff or database
    /// transaction logs). Much more efficient than snapshots for continuous sync.
    async fn generate_diff(&self, from_version: u64, to_version: u64) -> Result<DatabaseDiff>;

    // ========================================================================
    // REPLICA METHODS - Apply replication data to reconstruct state
    // ========================================================================

    /// Apply a snapshot to restore database state
    ///
    /// Restores from a full backup (like `pg_restore`), replacing the entire
    /// local database with the snapshot state.
    async fn apply_snapshot(&self, snapshot: DatabaseSnapshot) -> Result<()>;

    /// Apply incremental changes to update database state
    ///
    /// Applies a diff by executing the SQL statements contained within it,
    /// updating the local database incrementally.
    async fn apply_diff(&self, diff: DatabaseDiff) -> Result<()>;

    /// Get current database version
    async fn get_version(&self) -> Result<u64>;

    /// Set database version
    async fn set_version(&self, version: u64) -> Result<()>;
}

// ============================================================================
// SQLite Implementation
// ============================================================================

/// High-performance SQLite database implementation
pub struct SqliteDatabase {
    /// Connection pool for concurrent access
    pool: Pool<SqliteConnectionManager>,
    /// Current database version
    version: Arc<RwLock<u64>>,
    /// Performance statistics (deprecated - use metrics)
    stats: Arc<RwLock<PerformanceStats>>,
    /// Prepared statement cache
    prepared_statements: Arc<PreparedStatementCache>,
    /// Real-time metrics collector
    metrics: Arc<MetricsCollector>,
    /// Database file path
    db_path: String,
}

impl SqliteDatabase {
    /// Create a new SQLite database instance
    pub fn new<P: AsRef<Path>>(path: P, pool_size: u32) -> Result<Self> {
        let db_path = path.as_ref().to_string_lossy().to_string();
        let manager = SqliteConnectionManager::file(&db_path);

        let pool = Pool::builder()
            .max_size(pool_size)
            .build(manager)
            .map_err(|e| Error::Other(anyhow::anyhow!("Failed to create pool: {}", e)))?;

        // Initialize database with optimizations
        let conn = pool.get().map_err(|e| Error::Other(anyhow::anyhow!("{}", e)))?;
        Self::initialize_optimizations(&conn)?;
        Self::create_metadata_tables(&conn)?;

        // Initialize prepared statement cache
        let prepared_statements = Arc::new(PreparedStatementCache::new());
        prepared_statements.initialize_common();

        // Initialize metrics collector
        let metrics = Arc::new(MetricsCollector::new());

        info!("SQLite database initialized at {}", db_path);

        Ok(Self {
            pool,
            version: Arc::new(RwLock::new(0)),
            stats: Arc::new(RwLock::new(PerformanceStats::default())),
            prepared_statements,
            metrics,
            db_path,
        })
    }

    /// Get the prepared statement cache
    pub fn prepared_statements(&self) -> &PreparedStatementCache {
        &self.prepared_statements
    }

    /// Get the metrics collector
    pub fn metrics(&self) -> &MetricsCollector {
        &self.metrics
    }

    /// Initialize SQLite with maximum performance optimizations
    fn initialize_optimizations(conn: &Connection) -> Result<()> {
        info!("Applying SQLite performance optimizations");

        // WAL mode for concurrent reads during writes
        conn.pragma_update(None, "journal_mode", "WAL")?;

        // NORMAL: Ensures durability to OS, but not to disk on every write
        // Safe because we commit state to blockchain periodically
        conn.pragma_update(None, "synchronous", "NORMAL")?;

        // 2GB cache - keep hot data in RAM (negative = size in KB)
        conn.pragma_update(None, "cache_size", -2000000)?;

        // 256GB memory map - map entire DB file to virtual memory if possible
        conn.pragma_update(None, "mmap_size", 274877906944i64)?;

        // Keep temp tables/indices in memory
        conn.pragma_update(None, "temp_store", "MEMORY")?;

        // EXCLUSIVE mode - single sequencer doesn't need to coordinate
        conn.pragma_update(None, "locking_mode", "EXCLUSIVE")?;

        // 64KB pages (max size) - reduces B-tree depth for large datasets
        conn.pragma_update(None, "page_size", 65536)?;

        // WAL optimizations for write performance
        conn.pragma_update(None, "wal_autocheckpoint", 10000)?; // 10k pages before checkpoint

        // Optimize for SSDs - incremental vacuum
        conn.pragma_update(None, "auto_vacuum", "INCREMENTAL")?;

        debug!("SQLite optimizations applied successfully");
        Ok(())
    }

    /// Create internal metadata tables for version tracking
    fn create_metadata_tables(conn: &Connection) -> Result<()> {
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS _synddb_metadata (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            INSERT OR IGNORE INTO _synddb_metadata (key, value) VALUES ('version', '0');
            "#,
        )?;

        Ok(())
    }

    /// Get a connection from the pool
    fn get_connection(&self) -> Result<PooledConnection<SqliteConnectionManager>> {
        self.pool
            .get()
            .map_err(|e| Error::Other(anyhow::anyhow!("Failed to get connection: {}", e)))
    }

    /// Convert SqlValue to rusqlite parameter
    fn sql_value_to_param(value: &SqlValue) -> Box<dyn ToSql> {
        match value {
            SqlValue::Null => Box::new(rusqlite::types::Null),
            SqlValue::Integer(i) => Box::new(*i),
            SqlValue::Real(f) => Box::new(*f),
            SqlValue::Text(s) => Box::new(s.clone()),
            SqlValue::Blob(b) => Box::new(b.clone()),
        }
    }
}

#[async_trait]
impl SyndDatabase for SqliteDatabase {
    async fn begin_database_transaction(&self) -> Result<DatabaseTransaction> {
        let version = *self.version.read().await;
        Ok(DatabaseTransaction {
            id: uuid::Uuid::new_v4().to_string(),
            version,
        })
    }

    async fn execute(&self, sql: &str, params: Vec<SqlValue>) -> Result<ExecuteResult> {
        let start = Instant::now();
        let conn = self.get_connection()?;

        // Convert SqlValue params to rusqlite params
        let param_values: Vec<Box<dyn ToSql>> = params
            .iter()
            .map(|v| Self::sql_value_to_param(v))
            .collect();

        let param_refs: Vec<&dyn ToSql> = param_values
            .iter()
            .map(|p| p.as_ref())
            .collect();

        let rows_affected = conn.execute(sql, param_refs.as_slice())?;
        let last_insert_rowid = if rows_affected > 0 {
            Some(conn.last_insert_rowid())
        } else {
            None
        };

        let duration = start.elapsed();

        // Record metrics
        self.metrics.record_latency(duration);

        Ok(ExecuteResult {
            rows_affected,
            last_insert_rowid,
            duration,
        })
    }

    async fn execute_batch(&self, operations: Vec<SqlOperation>) -> Result<BatchResult> {
        let start = Instant::now();
        let mut conn = self.get_connection()?;

        // Begin transaction
        let tx = conn
            .transaction()
            .map_err(|e| Error::Database(e))?;

        let mut results = Vec::new();

        // Execute each operation
        for op in operations {
            let op_start = Instant::now();

            // Convert SqlValue params to ToSql trait objects
            let params: Vec<Box<dyn ToSql>> = op
                .params
                .iter()
                .map(Self::sql_value_to_param)
                .collect();

            // Convert to references for execute
            let param_refs: Vec<&dyn ToSql> = params
                .iter()
                .map(|p| p.as_ref())
                .collect();

            let rows_affected = tx.execute(&op.sql, param_refs.as_slice())?;
            let last_insert_rowid = if rows_affected > 0 {
                Some(tx.last_insert_rowid())
            } else {
                None
            };

            results.push(ExecuteResult {
                rows_affected,
                last_insert_rowid,
                duration: op_start.elapsed(),
            });
        }

        // Commit transaction
        tx.commit()?;

        let duration = start.elapsed();

        // Record metrics for batch operation
        self.metrics.record_latency(duration);

        Ok(BatchResult {
            success: true,
            results,
            duration,
        })
    }

    async fn query(&self, sql: &str, params: Vec<SqlValue>) -> Result<QueryResult> {
        let start = Instant::now();
        let conn = self.get_connection()?;

        let mut stmt = conn.prepare(sql)?;

        // Get column names
        let columns: Vec<String> = stmt
            .column_names()
            .iter()
            .map(|s| s.to_string())
            .collect();

        // Convert SqlValue params to rusqlite params
        let param_values: Vec<Box<dyn ToSql>> = params
            .iter()
            .map(|v| Self::sql_value_to_param(v))
            .collect();

        let param_refs: Vec<&dyn ToSql> = param_values
            .iter()
            .map(|p| p.as_ref())
            .collect();

        // Execute query and collect rows
        let mut rows_data = Vec::new();
        let mut rows = stmt.query(param_refs.as_slice())?;

        while let Some(row) = rows.next()? {
            let mut row_values = Vec::new();
            for i in 0..columns.len() {
                let value = match row.get_ref(i)? {
                    rusqlite::types::ValueRef::Null => SqlValue::Null,
                    rusqlite::types::ValueRef::Integer(i) => SqlValue::Integer(i),
                    rusqlite::types::ValueRef::Real(f) => SqlValue::Real(f),
                    rusqlite::types::ValueRef::Text(s) => {
                        SqlValue::Text(String::from_utf8_lossy(s).to_string())
                    }
                    rusqlite::types::ValueRef::Blob(b) => SqlValue::Blob(b.to_vec()),
                };
                row_values.push(value);
            }
            rows_data.push(row_values);
        }

        let row_count = rows_data.len();
        let duration = start.elapsed();

        // Record metrics for query
        self.metrics.record_latency(duration);

        Ok(QueryResult {
            columns,
            rows: rows_data,
            row_count,
            duration,
        })
    }

    async fn generate_snapshot(&self) -> Result<DatabaseSnapshot> {
        let version = *self.version.read().await;
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        info!("Generating database snapshot at version {}", version);

        // Checkpoint the WAL to ensure all data is in the main database file
        let conn = self.get_connection()?;
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
        drop(conn);

        // Read the entire database file
        let data = tokio::fs::read(&self.db_path).await?;
        let uncompressed_size = data.len();

        // Compress the database
        let compressed = zstd::encode_all(&data[..], 6)?;
        let compressed_size = compressed.len();

        // Calculate checksum
        let checksum = calculate_checksum(&compressed);

        info!(
            "Snapshot generated: {} -> {} bytes ({:.2}% compression)",
            uncompressed_size,
            compressed_size,
            (compressed_size as f64 / uncompressed_size as f64) * 100.0
        );

        Ok(DatabaseSnapshot {
            version,
            data: compressed,
            uncompressed_size,
            compressed_size,
            checksum,
            timestamp,
        })
    }

    async fn generate_diff(&self, from_version: u64, to_version: u64) -> Result<DatabaseDiff> {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        info!("Generating diff from version {} to {}", from_version, to_version);

        // TODO: Implement actual diff generation using WAL or change tracking
        // For now, this is a placeholder that would need to track changes
        let statements = Vec::new();

        // Serialize and compress statements
        let json = serde_json::to_vec(&statements)?;
        let compressed = zstd::encode_all(&json[..], 3)?;
        let compressed_size = compressed.len();
        let compression_ratio = if json.len() > 0 {
            compressed_size as f64 / json.len() as f64
        } else {
            0.0
        };

        let checksum = calculate_checksum(&compressed);

        Ok(DatabaseDiff {
            from_version,
            to_version,
            statements,
            compressed,
            compressed_size,
            compression_ratio,
            checksum,
            timestamp,
        })
    }

    async fn apply_snapshot(&self, snapshot: DatabaseSnapshot) -> Result<()> {
        info!("Applying snapshot at version {}", snapshot.version);

        // Verify checksum
        let calculated_checksum = calculate_checksum(&snapshot.data);
        if calculated_checksum != snapshot.checksum {
            return Err(Error::State("Snapshot checksum mismatch".to_string()));
        }

        // Decompress
        let decompressed = zstd::decode_all(&snapshot.data[..])?;

        // Write to a temporary file first
        let temp_path = format!("{}.tmp", self.db_path);
        tokio::fs::write(&temp_path, decompressed).await?;

        // TODO: Close existing connections and replace database file
        // For now, we'll just note that this needs proper connection draining
        // In production, this would involve:
        // 1. Stopping new connections
        // 2. Draining existing connections
        // 3. Replacing the file
        // 4. Reopening connections
        tokio::fs::rename(&temp_path, &self.db_path).await?;

        // Update version
        let mut version = self.version.write().await;
        *version = snapshot.version;

        info!("Snapshot applied successfully");
        Ok(())
    }

    async fn apply_diff(&self, diff: DatabaseDiff) -> Result<()> {
        info!("Applying diff from version {} to {}", diff.from_version, diff.to_version);

        // Verify checksum
        let calculated_checksum = calculate_checksum(&diff.compressed);
        if calculated_checksum != diff.checksum {
            return Err(Error::State("Diff checksum mismatch".to_string()));
        }

        // Execute all statements synchronously (no await points while holding connection)
        {
            let mut conn = self.get_connection()?;
            let tx = conn.transaction()?;

            // Execute all statements in the diff
            for statement in &diff.statements {
                tx.execute(&statement, [])?;
            }

            tx.commit()?;
        }

        // Update version after transaction is complete
        let mut version = self.version.write().await;
        *version = diff.to_version;

        info!("Diff applied successfully");
        Ok(())
    }

    async fn get_version(&self) -> Result<u64> {
        Ok(*self.version.read().await)
    }

    async fn set_version(&self, version: u64) -> Result<()> {
        let mut v = self.version.write().await;
        *v = version;

        // Also persist to metadata table
        let conn = self.get_connection()?;
        conn.execute(
            "UPDATE _synddb_metadata SET value = ?1 WHERE key = 'version'",
            [version.to_string()],
        )?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_database_creation() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");

        let db = SqliteDatabase::new(db_path, 4).unwrap();
        let version = db.get_version().await.unwrap();
        assert_eq!(version, 0);
    }

    #[tokio::test]
    async fn test_version_management() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");

        let db = SqliteDatabase::new(db_path, 4).unwrap();
        db.set_version(42).await.unwrap();

        let version = db.get_version().await.unwrap();
        assert_eq!(version, 42);
    }

    #[tokio::test]
    async fn test_execute_query() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");

        let db = SqliteDatabase::new(db_path, 4).unwrap();

        // Create table
        db.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, name TEXT)", vec![])
            .await
            .unwrap();

        // Insert data
        db.execute(
            "INSERT INTO test (name) VALUES (?1)",
            vec![SqlValue::Text("Alice".to_string())]
        )
        .await
        .unwrap();

        // Query data
        let result = db.query("SELECT * FROM test", vec![]).await.unwrap();
        assert_eq!(result.row_count, 1);
        assert_eq!(result.columns, vec!["id", "name"]);
    }
}
