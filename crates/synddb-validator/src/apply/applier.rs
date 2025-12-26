//! Changeset applier for reconstructing state from sequenced messages

use std::{
    fs,
    io::{Cursor, Read, Write},
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use anyhow::{Context, Result};
use rusqlite::{session::ConflictAction, Connection};
use synddb_shared::types::{
    message::{MessageType, SignedMessage},
    payloads::{ChangesetBatchRequest, SnapshotRequest, WithdrawalRequest},
};
use tracing::{debug, error, info, warn};

use crate::error::ValidatorError;

/// Database type for changeset application
enum DatabaseType {
    /// File-based database with path for snapshot restoration
    File(PathBuf),
    /// In-memory database
    InMemory,
}

/// Applies changesets from sequenced messages to an `SQLite` database
pub struct ChangesetApplier {
    /// The underlying database connection
    pub conn: Connection,
    /// Database type (file or in-memory)
    db_type: DatabaseType,
}

impl ChangesetApplier {
    /// Create a new applier with a database connection
    pub fn new(db_path: &str) -> Result<Self> {
        let conn =
            Connection::open(db_path).context(format!("Failed to open database: {db_path}"))?;

        // Set pragmas for performance
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")
            .context("Failed to set pragmas")?;

        info!(path = db_path, "Database opened for changeset application");

        Ok(Self {
            conn,
            db_type: DatabaseType::File(PathBuf::from(db_path)),
        })
    }

    /// Create an applier with an in-memory database (for testing)
    pub fn in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().context("Failed to open in-memory database")?;

        Ok(Self {
            conn,
            db_type: DatabaseType::InMemory,
        })
    }

    /// Apply a signed message to the database
    ///
    /// Handles different message types:
    /// - `Changeset`: Decompress and apply changeset batch
    /// - `Snapshot`: Restore database from snapshot
    /// - `Withdrawal`: Log only (no database changes)
    pub fn apply_message(&mut self, message: &SignedMessage) -> Result<()> {
        self.apply_message_with_rules(message, None)
    }

    /// Apply a signed message with optional validation rules
    ///
    /// If rules are provided, they are run after the changeset is applied
    /// but before the transaction is committed. If any rule fails,
    /// the transaction is rolled back and an error is returned.
    ///
    /// # Arguments
    ///
    /// * `message` - The signed message to apply
    /// * `rules` - Optional validation rules to run after applying the changeset
    pub fn apply_message_with_rules(
        &mut self,
        message: &SignedMessage,
        rules: Option<&crate::rules::RuleRegistry>,
    ) -> Result<()> {
        match message.message_type {
            MessageType::Changeset => self.apply_changeset_message_with_rules(message, rules),
            MessageType::Snapshot => self.apply_snapshot_message(message),
            MessageType::Withdrawal => {
                debug!(
                    sequence = message.sequence,
                    "Withdrawal message (no database changes)"
                );
                Ok(())
            }
        }
    }

    /// Decompress and parse a CBOR message payload
    fn decompress_and_parse<T: serde::de::DeserializeOwned>(
        &self,
        message: &SignedMessage,
        type_name: &str,
    ) -> Result<T> {
        let decompressed = Self::decompress(&message.payload).map_err(|e| {
            ValidatorError::DecompressionError(format!(
                "Failed to decompress {type_name} at sequence {}: {e}",
                message.sequence
            ))
        })?;

        ciborium::from_reader(decompressed.as_slice()).map_err(|e| {
            ValidatorError::ParseError(format!(
                "Failed to parse {type_name} at sequence {}: {e}",
                message.sequence
            ))
            .into()
        })
    }

    /// Apply a changeset message with optional validation rules
    fn apply_changeset_message_with_rules(
        &mut self,
        message: &SignedMessage,
        rules: Option<&crate::rules::RuleRegistry>,
    ) -> Result<()> {
        let batch: ChangesetBatchRequest = self.decompress_and_parse(message, "changeset batch")?;

        debug!(
            sequence = message.sequence,
            batch_id = %batch.batch_id,
            changeset_count = batch.changesets.len(),
            "Applying changeset batch"
        );

        // Diagnostic logging: Check database state before applying changesets
        if let Ok(tables) = self.get_table_row_counts() {
            debug!(
                sequence = message.sequence,
                tables = ?tables,
                "Database state before applying changeset"
            );
        }

        let tx = self.conn.transaction().map_err(|e| {
            error!(error = %e, "Failed to begin transaction");
            ValidatorError::DatabaseError(format!("Failed to begin transaction: {e}"))
        })?;

        for (i, changeset) in batch.changesets.iter().enumerate() {
            if changeset.data.is_empty() {
                debug!(
                    sequence = message.sequence,
                    index = i,
                    "Skipping empty changeset"
                );
                continue;
            }

            Self::apply_single_changeset(&tx, &changeset.data).map_err(|e| {
                ValidatorError::ChangesetApplyError {
                    sequence: message.sequence,
                    reason: format!("Changeset {} in batch {}: {e}", i, batch.batch_id),
                }
            })?;
        }

        // Run validation rules before committing (if any)
        if let Some(registry) = rules {
            registry.validate_all(&tx, message.sequence)?;
        }

        tx.commit().map_err(|e| {
            error!(error = %e, "Failed to commit transaction");
            ValidatorError::DatabaseError(format!("Failed to commit transaction: {e}"))
        })?;

        info!(
            sequence = message.sequence,
            batch_id = %batch.batch_id,
            "Changeset batch applied successfully"
        );

        Ok(())
    }

    /// Apply a snapshot message by restoring the database from the snapshot
    fn apply_snapshot_message(&mut self, message: &SignedMessage) -> Result<()> {
        let request: SnapshotRequest = self.decompress_and_parse(message, "snapshot")?;

        info!(
            sequence = message.sequence,
            message_id = %request.message_id,
            snapshot_sequence = request.snapshot.sequence,
            size = request.snapshot.data.len(),
            "Restoring database from snapshot"
        );

        self.restore_snapshot(&request.snapshot.data)?;

        // Diagnostic logging: Check database state after restore
        if let Ok(tables) = self.get_table_row_counts() {
            debug!(
                sequence = message.sequence,
                tables = ?tables,
                "Database state after snapshot restore"
            );
        }

        info!(
            sequence = message.sequence,
            message_id = %request.message_id,
            "Snapshot restored successfully"
        );

        Ok(())
    }

    /// Restore a snapshot to the database
    ///
    /// Uses `SQLite`'s backup API to atomically restore the database.
    /// Handles both file-based and in-memory databases.
    fn restore_snapshot(&mut self, snapshot_data: &[u8]) -> Result<()> {
        // Generate temp file path based on database type
        let temp_path = match &self.db_type {
            DatabaseType::File(path) => PathBuf::from(format!("{}.snapshot.tmp", path.display())),
            DatabaseType::InMemory => {
                static COUNTER: AtomicU64 = AtomicU64::new(0);
                let id = COUNTER.fetch_add(1, Ordering::SeqCst);
                let pid = std::process::id();
                std::env::temp_dir().join(format!("synddb_snapshot_restore_{pid}_{id}.db"))
            }
        };

        // Write snapshot to temporary file
        {
            let mut file =
                fs::File::create(&temp_path).context("Failed to create temporary snapshot file")?;
            file.write_all(snapshot_data)
                .context("Failed to write snapshot data")?;
            // Sync for file-based databases to ensure durability
            if matches!(self.db_type, DatabaseType::File(_)) {
                file.sync_all().context("Failed to sync snapshot file")?;
            }
        }

        // Open the snapshot as a source database
        let source = Connection::open(&temp_path).context("Failed to open snapshot database")?;

        // Verify it's a valid SQLite database
        source
            .query_row("SELECT 1", [], |_| Ok(()))
            .context("Snapshot is not a valid SQLite database")?;

        // Use SQLite backup API to restore
        {
            let backup = rusqlite::backup::Backup::new(&source, &mut self.conn)
                .context("Failed to create backup handle")?;

            backup
                .run_to_completion(100, std::time::Duration::from_millis(10), None)
                .context("Failed to restore from snapshot")?;
        }

        // Clean up temporary file
        if let Err(e) = fs::remove_file(&temp_path) {
            warn!(path = %temp_path.display(), error = %e, "Failed to remove temporary snapshot file");
        }

        // Re-apply WAL mode for file-based databases after restore
        if matches!(self.db_type, DatabaseType::File(_)) {
            self.conn
                .execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")
                .context("Failed to re-apply pragmas after snapshot restore")?;
        }

        Ok(())
    }

    /// Extract snapshot request from a message
    ///
    /// Returns `Some(SnapshotRequest)` if this is a snapshot message,
    /// `None` otherwise.
    pub fn extract_snapshot(message: &SignedMessage) -> Result<Option<SnapshotRequest>> {
        if message.message_type != MessageType::Snapshot {
            return Ok(None);
        }

        let decompressed = Self::decompress(&message.payload)?;
        let request: SnapshotRequest = ciborium::from_reader(decompressed.as_slice())
            .context("Failed to parse snapshot request")?;

        Ok(Some(request))
    }

    /// Decompress a zstd-compressed payload
    fn decompress(data: &[u8]) -> Result<Vec<u8>> {
        let mut decoder = zstd::Decoder::new(data).context("Failed to create zstd decoder")?;
        let mut decompressed = Vec::new();
        decoder
            .read_to_end(&mut decompressed)
            .context("Failed to decompress")?;
        Ok(decompressed)
    }

    /// Extract withdrawal request from a message
    ///
    /// Returns `Some(WithdrawalRequest)` if this is a withdrawal message,
    /// `None` otherwise.
    pub fn extract_withdrawal(message: &SignedMessage) -> Result<Option<WithdrawalRequest>> {
        if message.message_type != MessageType::Withdrawal {
            return Ok(None);
        }

        let decompressed = Self::decompress(&message.payload)?;
        let request: WithdrawalRequest = ciborium::from_reader(decompressed.as_slice())
            .context("Failed to parse withdrawal request")?;

        Ok(Some(request))
    }

    /// Get row counts for all user tables (for diagnostic logging)
    fn get_table_row_counts(&self) -> Result<Vec<(String, i64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'",
        )?;
        let table_names: Vec<String> = stmt
            .query_map([], |row| row.get(0))?
            .filter_map(|r| r.ok())
            .collect();

        let mut counts = Vec::new();
        for table in table_names {
            if let Ok(count) =
                self.conn
                    .query_row(&format!("SELECT COUNT(*) FROM \"{}\"", table), [], |row| {
                        row.get::<_, i64>(0)
                    })
            {
                counts.push((table, count));
            }
        }
        Ok(counts)
    }

    /// Apply a single changeset to the database using the streaming API.
    ///
    /// IMPORTANT: `SQLite`'s default behavior is to SILENTLY IGNORE:
    /// - Changesets for tables that don't exist
    /// - Columns that don't exist in the target table
    ///
    /// We validate schema compatibility before applying to prevent silent data loss.
    fn apply_single_changeset(conn: &Connection, data: &[u8]) -> Result<()> {
        // Validate schema compatibility before applying.
        // This prevents silent data loss from missing tables or column mismatches.
        Self::validate_changeset_schema(conn, data)?;

        let mut cursor = Cursor::new(data);

        conn.apply_strm(
            &mut cursor,
            None::<fn(&str) -> bool>,
            |conflict_type, _item| {
                warn!(
                    conflict_type = ?conflict_type,
                    "Changeset conflict detected, aborting"
                );
                ConflictAction::SQLITE_CHANGESET_ABORT
            },
        )
        .map_err(|e| anyhow::anyhow!("Failed to apply changeset: {e}"))?;

        Ok(())
    }

    /// Validate that a changeset can be applied to the target database.
    ///
    /// Checks for:
    /// 1. Missing tables - tables referenced in changeset that don't exist
    /// 2. Column count mismatch - tables with different number of columns
    ///
    /// Uses `SQLite`'s changeset iteration API to find all tables referenced.
    ///
    /// Returns `Ok(())` if valid, or an error describing the schema mismatch.
    fn validate_changeset_schema(conn: &Connection, data: &[u8]) -> Result<()> {
        use fallible_streaming_iterator::FallibleStreamingIterator;
        use rusqlite::session::ChangesetIter;
        use std::collections::HashMap;

        // Map of table name -> expected column count from changeset
        let mut table_columns: HashMap<String, i32> = HashMap::new();

        // Create a streaming iterator over the changeset
        let cursor: &mut dyn Read = &mut Cursor::new(data);
        let mut iter = ChangesetIter::start_strm(&cursor)
            .map_err(|e| anyhow::anyhow!("Failed to start changeset iterator: {e}"))?;

        // Iterate through changeset items to find all referenced tables
        loop {
            match iter.next() {
                Ok(Some(item)) => {
                    let op = item
                        .op()
                        .map_err(|e| anyhow::anyhow!("Failed to get changeset operation: {e}"))?;
                    let table_name = op.table_name().to_string();
                    let col_count = op.number_of_columns();

                    // Record the column count (all operations on same table should have same count)
                    table_columns.insert(table_name, col_count);
                }
                Ok(None) => break,
                Err(e) => {
                    return Err(anyhow::anyhow!("Failed to read changeset item: {e}"));
                }
            }
        }

        // Validate each table exists and has matching column count
        let mut missing_tables = Vec::new();
        let mut column_mismatches = Vec::new();

        for (table, expected_cols) in table_columns {
            // Check if table exists
            let table_info: Option<i32> = conn
                .query_row(
                    "SELECT COUNT(*) FROM pragma_table_info(?)",
                    [&table],
                    |row| row.get(0),
                )
                .ok();

            match table_info {
                None | Some(0) => {
                    missing_tables.push(table);
                }
                Some(actual_cols) => {
                    if actual_cols != expected_cols {
                        column_mismatches.push(format!(
                            "{} (changeset has {} columns, target has {})",
                            table, expected_cols, actual_cols
                        ));
                    }
                }
            }
        }

        // Report errors
        if !missing_tables.is_empty() {
            return Err(anyhow::anyhow!(
                "Changeset references tables that don't exist: {}. \
                 A snapshot is required to sync schema first.",
                missing_tables.join(", ")
            ));
        }

        if !column_mismatches.is_empty() {
            return Err(anyhow::anyhow!(
                "Schema mismatch - column counts don't match: {}. \
                 A snapshot is required to sync schema first.",
                column_mismatches.join("; ")
            ));
        }

        Ok(())
    }
}

impl std::fmt::Debug for ChangesetApplier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChangesetApplier")
            .field("conn", &"<Connection>")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::session::Session;

    /// Create a test changeset that modifies a table (returns raw bytes)
    fn create_test_changeset(source_conn: &Connection) -> Vec<u8> {
        // Create a session and attach to all tables
        let mut session = Session::new(source_conn).unwrap();
        session.attach(None::<&str>).unwrap();

        // Make a change
        source_conn
            .execute("UPDATE users SET name = 'Bob' WHERE id = 1", [])
            .unwrap();

        // Get the changeset as raw bytes
        let mut output = Vec::new();
        session.changeset_strm(&mut output).unwrap();
        output
    }

    /// Create a compressed changeset batch (CBOR format)
    fn create_compressed_batch(changesets: Vec<Vec<u8>>) -> Vec<u8> {
        use std::io::Write;

        let batch = ChangesetBatchRequest {
            batch_id: "test-batch".to_string(),
            changesets: changesets
                .into_iter()
                .enumerate()
                .map(|(i, data)| synddb_shared::types::payloads::ChangesetData {
                    data,
                    sequence: i as u64,
                    timestamp: 1700000000,
                })
                .collect(),
            attestation_token: None,
        };

        let mut cbor = Vec::new();
        ciborium::into_writer(&batch, &mut cbor).unwrap();

        // Compress with zstd
        let mut encoder = zstd::Encoder::new(Vec::new(), 3).unwrap();
        encoder.write_all(&cbor).unwrap();
        encoder.finish().unwrap()
    }

    #[test]
    fn test_apply_empty_changeset_batch() {
        let mut applier = ChangesetApplier::in_memory().unwrap();

        // Create an empty batch
        let compressed = create_compressed_batch(vec![]);

        let message = SignedMessage {
            sequence: 0,
            timestamp: 1700000000,
            message_type: MessageType::Changeset,
            payload: compressed,
            message_hash: "0x0".to_string(),
            signature: "0x0".to_string(),
            signer: "0x0".to_string(),
            cose_protected_header: vec![],
        };

        // Should succeed with no changes
        assert!(applier.apply_message(&message).is_ok());
    }

    #[test]
    fn test_changeset_roundtrip() {
        // Create source database with table and initial data
        let source = Connection::open_in_memory().unwrap();
        source
            .execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)", [])
            .unwrap();
        source
            .execute("INSERT INTO users VALUES (1, 'Alice')", [])
            .unwrap();

        // Create changeset from modification
        let changeset = create_test_changeset(&source);

        // Create target database with same schema and initial state
        let mut applier = ChangesetApplier::in_memory().unwrap();
        applier
            .conn
            .execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)", [])
            .unwrap();
        applier
            .conn
            .execute("INSERT INTO users VALUES (1, 'Alice')", [])
            .unwrap();

        // Create compressed batch
        let compressed = create_compressed_batch(vec![changeset]);

        let message = SignedMessage {
            sequence: 0,
            timestamp: 1700000000,
            message_type: MessageType::Changeset,
            payload: compressed,
            message_hash: "0x0".to_string(),
            signature: "0x0".to_string(),
            signer: "0x0".to_string(),
            cose_protected_header: vec![],
        };

        // Apply the changeset
        applier.apply_message(&message).unwrap();

        // Verify the change was applied
        let name: String = applier
            .conn
            .query_row("SELECT name FROM users WHERE id = 1", [], |row| row.get(0))
            .unwrap();
        assert_eq!(name, "Bob");
    }

    #[test]
    fn test_withdrawal_message_no_change() {
        let mut applier = ChangesetApplier::in_memory().unwrap();

        let message = SignedMessage {
            sequence: 0,
            timestamp: 1700000000,
            message_type: MessageType::Withdrawal,
            payload: vec![],
            message_hash: "0x0".to_string(),
            signature: "0x0".to_string(),
            signer: "0x0".to_string(),
            cose_protected_header: vec![],
        };

        // Should succeed with no changes
        assert!(applier.apply_message(&message).is_ok());
    }

    /// Create a compressed snapshot message (CBOR format)
    fn create_compressed_snapshot(db_data: Vec<u8>, sequence: u64) -> Vec<u8> {
        use std::io::Write;
        use synddb_shared::types::payloads::{SnapshotData, SnapshotRequest};

        let request = SnapshotRequest {
            snapshot: SnapshotData {
                data: db_data,
                timestamp: 1700000000,
                sequence,
            },
            message_id: format!("snap-{sequence}"),
            attestation_token: None,
        };

        let mut cbor = Vec::new();
        ciborium::into_writer(&request, &mut cbor).unwrap();

        // Compress with zstd
        let mut encoder = zstd::Encoder::new(Vec::new(), 3).unwrap();
        encoder.write_all(&cbor).unwrap();
        encoder.finish().unwrap()
    }

    /// Create bytes from a given connection using SQLite backup
    fn create_database_bytes_from(conn: &Connection) -> Vec<u8> {
        use std::{
            fs,
            sync::atomic::{AtomicU64, Ordering},
        };

        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let pid = std::process::id();

        let temp_path = std::env::temp_dir().join(format!("test_snapshot_from_{pid}_{id}.db"));
        let _ = fs::remove_file(&temp_path);

        {
            let mut dest = Connection::open(&temp_path).unwrap();
            dest.execute_batch("PRAGMA journal_mode=DELETE;").unwrap();

            let backup = rusqlite::backup::Backup::new(conn, &mut dest).unwrap();
            backup
                .run_to_completion(100, std::time::Duration::from_millis(10), None)
                .unwrap();
        }

        let bytes = fs::read(&temp_path).unwrap();
        let _ = fs::remove_file(&temp_path);
        bytes
    }

    /// Create an in-memory database and return its bytes
    fn create_test_database_bytes() -> Vec<u8> {
        use std::{
            fs,
            sync::atomic::{AtomicU64, Ordering},
        };

        // Use atomic counter + process ID for unique file names in parallel tests.
        // Process ID is critical for nextest which runs each test in a separate process,
        // causing static counters to reset and thread IDs to always be ThreadId(1).
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let pid = std::process::id();

        // Create a file-based database with some data (so we can read the bytes)
        let temp_path = std::env::temp_dir().join(format!("test_snapshot_source_{pid}_{id}.db"));
        let _ = fs::remove_file(&temp_path); // clean up from previous runs

        {
            let conn = Connection::open(&temp_path).unwrap();
            // Disable WAL mode for simpler snapshot format
            conn.execute_batch("PRAGMA journal_mode=DELETE;").unwrap();
            conn.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)", [])
                .unwrap();
            conn.execute("INSERT INTO users VALUES (1, 'Alice')", [])
                .unwrap();
            conn.execute("INSERT INTO users VALUES (2, 'Bob')", [])
                .unwrap();
        }

        // Read the file bytes
        let bytes = fs::read(&temp_path).unwrap();
        let _ = fs::remove_file(&temp_path);
        bytes
    }

    #[test]
    fn test_snapshot_restore_in_memory() {
        // Create a target applier (empty database)
        let mut applier = ChangesetApplier::in_memory().unwrap();

        // Create snapshot data from a test database
        let snapshot_data = create_test_database_bytes();
        let compressed = create_compressed_snapshot(snapshot_data, 100);

        let message = SignedMessage {
            sequence: 101,
            timestamp: 1700000000,
            message_type: MessageType::Snapshot,
            payload: compressed,
            message_hash: "0x0".to_string(),
            signature: "0x0".to_string(),
            signer: "0x0".to_string(),
            cose_protected_header: vec![],
        };

        // Apply the snapshot
        applier.apply_message(&message).unwrap();

        // Verify the data was restored
        let count: i64 = applier
            .conn
            .query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 2);

        let name: String = applier
            .conn
            .query_row("SELECT name FROM users WHERE id = 1", [], |row| row.get(0))
            .unwrap();
        assert_eq!(name, "Alice");
    }

    #[test]
    fn test_snapshot_restore_to_file() {
        use std::{
            fs,
            sync::atomic::{AtomicU64, Ordering},
        };

        // Create a file-based applier with unique path for parallel test execution.
        // Use both process ID and atomic counter for consistency with other temp file paths.
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let pid = std::process::id();
        let temp_path = std::env::temp_dir().join(format!("test_snapshot_target_{pid}_{id}.db"));
        let temp_path_str = temp_path.to_str().unwrap();

        // Clean up any previous test
        let _ = fs::remove_file(&temp_path);

        let mut applier = ChangesetApplier::new(temp_path_str).unwrap();

        // Create snapshot data from a test database
        let snapshot_data = create_test_database_bytes();
        let compressed = create_compressed_snapshot(snapshot_data, 100);

        let message = SignedMessage {
            sequence: 101,
            timestamp: 1700000000,
            message_type: MessageType::Snapshot,
            payload: compressed,
            message_hash: "0x0".to_string(),
            signature: "0x0".to_string(),
            signer: "0x0".to_string(),
            cose_protected_header: vec![],
        };

        // Apply the snapshot
        applier.apply_message(&message).unwrap();

        // Verify the data was restored
        let count: i64 = applier
            .conn
            .query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 2);

        // Clean up
        drop(applier);
        let _ = fs::remove_file(&temp_path);
    }

    #[test]
    fn test_snapshot_invalid_database() {
        let mut applier = ChangesetApplier::in_memory().unwrap();

        // Create a snapshot with invalid database data
        let invalid_data = b"not a valid sqlite database".to_vec();
        let compressed = create_compressed_snapshot(invalid_data, 100);

        let message = SignedMessage {
            sequence: 101,
            timestamp: 1700000000,
            message_type: MessageType::Snapshot,
            payload: compressed,
            message_hash: "0x0".to_string(),
            signature: "0x0".to_string(),
            signer: "0x0".to_string(),
            cose_protected_header: vec![],
        };

        // Should fail
        let result = applier.apply_message(&message);
        assert!(result.is_err());
    }

    #[test]
    fn test_extract_snapshot() {
        let snapshot_data = create_test_database_bytes();
        let compressed = create_compressed_snapshot(snapshot_data.clone(), 42);

        let message = SignedMessage {
            sequence: 43,
            timestamp: 1700000000,
            message_type: MessageType::Snapshot,
            payload: compressed,
            message_hash: "0x0".to_string(),
            signature: "0x0".to_string(),
            signer: "0x0".to_string(),
            cose_protected_header: vec![],
        };

        let extracted = ChangesetApplier::extract_snapshot(&message).unwrap();
        assert!(extracted.is_some());

        let request = extracted.unwrap();
        assert_eq!(request.message_id, "snap-42");
        assert_eq!(request.snapshot.sequence, 42);
        assert_eq!(request.snapshot.data.len(), snapshot_data.len());
    }

    #[test]
    fn test_extract_snapshot_wrong_type() {
        let message = SignedMessage {
            sequence: 0,
            timestamp: 1700000000,
            message_type: MessageType::Changeset,
            payload: vec![],
            message_hash: "0x0".to_string(),
            signature: "0x0".to_string(),
            signer: "0x0".to_string(),
            cose_protected_header: vec![],
        };

        let extracted = ChangesetApplier::extract_snapshot(&message).unwrap();
        assert!(extracted.is_none());
    }

    #[test]
    fn test_invalid_compressed_payload() {
        let mut applier = ChangesetApplier::in_memory().unwrap();

        let message = SignedMessage {
            sequence: 0,
            timestamp: 1700000000,
            message_type: MessageType::Changeset,
            payload: vec![0, 1, 2, 3], // Invalid zstd data
            message_hash: "0x0".to_string(),
            signature: "0x0".to_string(),
            signer: "0x0".to_string(),
            cose_protected_header: vec![],
        };

        let result = applier.apply_message(&message);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("decompress") || err.contains("zstd"));
    }

    #[test]
    fn test_multiple_changesets_in_batch() {
        // Create source database
        let source = Connection::open_in_memory().unwrap();
        source
            .execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)", [])
            .unwrap();
        source
            .execute("INSERT INTO users VALUES (1, 'Alice')", [])
            .unwrap();
        source
            .execute("INSERT INTO users VALUES (2, 'Charlie')", [])
            .unwrap();

        // Create first changeset
        let mut session1 = Session::new(&source).unwrap();
        session1.attach(None::<&str>).unwrap();
        source
            .execute("UPDATE users SET name = 'Bob' WHERE id = 1", [])
            .unwrap();
        let mut changeset1 = Vec::new();
        session1.changeset_strm(&mut changeset1).unwrap();

        // Create second changeset
        let mut session2 = Session::new(&source).unwrap();
        session2.attach(None::<&str>).unwrap();
        source
            .execute("UPDATE users SET name = 'Dave' WHERE id = 2", [])
            .unwrap();
        let mut changeset2 = Vec::new();
        session2.changeset_strm(&mut changeset2).unwrap();

        // Create target database
        let mut applier = ChangesetApplier::in_memory().unwrap();
        applier
            .conn
            .execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)", [])
            .unwrap();
        applier
            .conn
            .execute("INSERT INTO users VALUES (1, 'Alice')", [])
            .unwrap();
        applier
            .conn
            .execute("INSERT INTO users VALUES (2, 'Charlie')", [])
            .unwrap();

        // Create batch with both changesets
        let compressed = create_compressed_batch(vec![changeset1, changeset2]);

        let message = SignedMessage {
            sequence: 0,
            timestamp: 1700000000,
            message_type: MessageType::Changeset,
            payload: compressed,
            message_hash: "0x0".to_string(),
            signature: "0x0".to_string(),
            signer: "0x0".to_string(),
            cose_protected_header: vec![],
        };

        // Apply the batch
        applier.apply_message(&message).unwrap();

        // Verify both changes were applied
        let name1: String = applier
            .conn
            .query_row("SELECT name FROM users WHERE id = 1", [], |row| row.get(0))
            .unwrap();
        let name2: String = applier
            .conn
            .query_row("SELECT name FROM users WHERE id = 2", [], |row| row.get(0))
            .unwrap();

        assert_eq!(name1, "Bob");
        assert_eq!(name2, "Dave");
    }

    // =========================================================================
    // Schema Mismatch Tests
    // =========================================================================
    //
    // These tests verify that changeset application fails appropriately when
    // the target database schema doesn't match the source. This documents
    // the failure modes that occur when DDL changes aren't properly synced.

    #[test]
    fn test_changeset_missing_table() {
        // When a changeset references a table that doesn't exist,
        // the application should fail with an error.

        // Create source database with table
        let source = Connection::open_in_memory().unwrap();
        source
            .execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)", [])
            .unwrap();
        source
            .execute("INSERT INTO users VALUES (1, 'Alice')", [])
            .unwrap();

        // Create changeset (UPDATE on users table)
        let changeset = create_test_changeset(&source);
        assert!(!changeset.is_empty(), "Changeset should have content");

        // Create target database WITHOUT the users table
        let mut applier = ChangesetApplier::in_memory().unwrap();
        // Note: NOT creating the users table

        let compressed = create_compressed_batch(vec![changeset]);
        let message = SignedMessage {
            sequence: 0,
            timestamp: 1700000000,
            message_type: MessageType::Changeset,
            payload: compressed,
            message_hash: "0x0".to_string(),
            signature: "0x0".to_string(),
            signer: "0x0".to_string(),
            cose_protected_header: vec![],
        };

        // Application should fail
        let result = applier.apply_message(&message);
        assert!(result.is_err(), "Should fail when table doesn't exist");

        let err = result.unwrap_err();
        let err_str = err.to_string();
        assert!(
            err_str.contains("Failed to apply changeset")
                || err_str.contains("no such table")
                || err_str.contains("Changeset"),
            "Error should mention changeset failure: {}",
            err_str
        );
    }

    #[test]
    fn test_changeset_insert_missing_table() {
        // INSERT to a table that doesn't exist should fail

        // Create source database with table
        let source = Connection::open_in_memory().unwrap();
        source
            .execute(
                "CREATE TABLE orders (id INTEGER PRIMARY KEY, amount INTEGER)",
                [],
            )
            .unwrap();

        // Create session and capture INSERT
        let mut session = Session::new(&source).unwrap();
        session.attach(None::<&str>).unwrap();

        source
            .execute("INSERT INTO orders VALUES (1, 100)", [])
            .unwrap();

        let mut changeset = Vec::new();
        session.changeset_strm(&mut changeset).unwrap();
        assert!(!changeset.is_empty());

        // Target has no orders table
        let mut applier = ChangesetApplier::in_memory().unwrap();

        let compressed = create_compressed_batch(vec![changeset]);
        let message = SignedMessage {
            sequence: 0,
            timestamp: 1700000000,
            message_type: MessageType::Changeset,
            payload: compressed,
            message_hash: "0x0".to_string(),
            signature: "0x0".to_string(),
            signer: "0x0".to_string(),
            cose_protected_header: vec![],
        };

        let result = applier.apply_message(&message);
        assert!(result.is_err(), "INSERT to missing table should fail");
    }

    #[test]
    fn test_changeset_delete_missing_table() {
        // DELETE from a table that doesn't exist should fail

        // Create source database with table and data
        let source = Connection::open_in_memory().unwrap();
        source
            .execute("CREATE TABLE items (id INTEGER PRIMARY KEY)", [])
            .unwrap();
        source.execute("INSERT INTO items VALUES (1)", []).unwrap();

        // Create session and capture DELETE
        let mut session = Session::new(&source).unwrap();
        session.attach(None::<&str>).unwrap();

        source
            .execute("DELETE FROM items WHERE id = 1", [])
            .unwrap();

        let mut changeset = Vec::new();
        session.changeset_strm(&mut changeset).unwrap();
        assert!(!changeset.is_empty());

        // Target has no items table
        let mut applier = ChangesetApplier::in_memory().unwrap();

        let compressed = create_compressed_batch(vec![changeset]);
        let message = SignedMessage {
            sequence: 0,
            timestamp: 1700000000,
            message_type: MessageType::Changeset,
            payload: compressed,
            message_hash: "0x0".to_string(),
            signature: "0x0".to_string(),
            signer: "0x0".to_string(),
            cose_protected_header: vec![],
        };

        let result = applier.apply_message(&message);
        assert!(result.is_err(), "DELETE from missing table should fail");
    }

    #[test]
    fn test_changeset_preimage_mismatch() {
        // When an UPDATE's pre-image doesn't match the target database,
        // application should fail (data out of sync).

        // Create source database
        let source = Connection::open_in_memory().unwrap();
        source
            .execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)", [])
            .unwrap();
        source
            .execute("INSERT INTO users VALUES (1, 'Alice')", [])
            .unwrap();

        // Create changeset: UPDATE Alice -> Bob
        let changeset = create_test_changeset(&source);

        // Create target with DIFFERENT initial data
        let mut applier = ChangesetApplier::in_memory().unwrap();
        applier
            .conn
            .execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)", [])
            .unwrap();
        // Insert 'Charlie' instead of 'Alice' - pre-image won't match
        applier
            .conn
            .execute("INSERT INTO users VALUES (1, 'Charlie')", [])
            .unwrap();

        let compressed = create_compressed_batch(vec![changeset]);
        let message = SignedMessage {
            sequence: 0,
            timestamp: 1700000000,
            message_type: MessageType::Changeset,
            payload: compressed,
            message_hash: "0x0".to_string(),
            signature: "0x0".to_string(),
            signer: "0x0".to_string(),
            cose_protected_header: vec![],
        };

        // Should fail due to pre-image mismatch (conflict)
        let result = applier.apply_message(&message);
        assert!(
            result.is_err(),
            "Should fail when pre-image doesn't match (data out of sync)"
        );
    }

    #[test]
    fn test_changeset_row_does_not_exist() {
        // When an UPDATE/DELETE targets a row that doesn't exist,
        // application should fail.

        // Create source database with data
        let source = Connection::open_in_memory().unwrap();
        source
            .execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)", [])
            .unwrap();
        source
            .execute("INSERT INTO users VALUES (1, 'Alice')", [])
            .unwrap();

        // Create changeset: UPDATE row 1
        let changeset = create_test_changeset(&source);

        // Create target with schema but NO data
        let mut applier = ChangesetApplier::in_memory().unwrap();
        applier
            .conn
            .execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)", [])
            .unwrap();
        // No INSERT - row doesn't exist

        let compressed = create_compressed_batch(vec![changeset]);
        let message = SignedMessage {
            sequence: 0,
            timestamp: 1700000000,
            message_type: MessageType::Changeset,
            payload: compressed,
            message_hash: "0x0".to_string(),
            signature: "0x0".to_string(),
            signer: "0x0".to_string(),
            cose_protected_header: vec![],
        };

        // Should fail - can't update a row that doesn't exist
        let result = applier.apply_message(&message);
        assert!(result.is_err(), "Should fail when row doesn't exist");
    }

    #[test]
    fn test_changeset_wrong_table_schema() {
        // When the table exists but has a different number of columns,
        // we detect this BEFORE applying the changeset and reject it.
        //
        // This prevents SQLite's default behavior of silently ignoring
        // columns that don't exist in the target table.

        // Create source database with columns: id, name, email (3 columns)
        let source = Connection::open_in_memory().unwrap();
        source
            .execute(
                "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT, email TEXT)",
                [],
            )
            .unwrap();
        source
            .execute(
                "INSERT INTO users VALUES (1, 'Alice', 'alice@test.com')",
                [],
            )
            .unwrap();

        // Create session and capture UPDATE to email column
        let mut session = Session::new(&source).unwrap();
        session.attach(None::<&str>).unwrap();

        source
            .execute("UPDATE users SET email = 'bob@test.com' WHERE id = 1", [])
            .unwrap();

        let mut changeset = Vec::new();
        session.changeset_strm(&mut changeset).unwrap();
        assert!(!changeset.is_empty());

        // Create target with only 2 columns (missing email)
        let mut applier = ChangesetApplier::in_memory().unwrap();
        applier
            .conn
            .execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)", [])
            .unwrap();
        applier
            .conn
            .execute("INSERT INTO users VALUES (1, 'Alice')", [])
            .unwrap();

        let compressed = create_compressed_batch(vec![changeset]);
        let message = SignedMessage {
            sequence: 0,
            timestamp: 1700000000,
            message_type: MessageType::Changeset,
            payload: compressed,
            message_hash: "0x0".to_string(),
            signature: "0x0".to_string(),
            signer: "0x0".to_string(),
            cose_protected_header: vec![],
        };

        // Should fail - column count mismatch detected
        let result = applier.apply_message(&message);
        assert!(
            result.is_err(),
            "Should fail when column counts don't match"
        );

        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("column") || err.contains("Schema mismatch"),
            "Error should mention column mismatch: {}",
            err
        );
    }

    #[test]
    fn test_snapshot_then_changeset_works() {
        // Verify that after a snapshot, subsequent changesets can be applied.
        // This is the "recovery" path.

        // Create source database
        let source = Connection::open_in_memory().unwrap();
        source
            .execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)", [])
            .unwrap();
        source
            .execute("INSERT INTO users VALUES (1, 'Alice')", [])
            .unwrap();

        // Create a snapshot of the source
        let snapshot_data = create_database_bytes_from(&source);

        // Create changeset: UPDATE Alice -> Bob
        let changeset = create_test_changeset(&source);

        // Create empty target (no schema)
        let mut applier = ChangesetApplier::in_memory().unwrap();

        // First, apply snapshot
        let snapshot_compressed = create_compressed_snapshot(snapshot_data, 0);
        let snapshot_message = SignedMessage {
            sequence: 0,
            timestamp: 1700000000,
            message_type: MessageType::Snapshot,
            payload: snapshot_compressed,
            message_hash: "0x0".to_string(),
            signature: "0x0".to_string(),
            signer: "0x0".to_string(),
            cose_protected_header: vec![],
        };

        applier.apply_message(&snapshot_message).unwrap();

        // Now changeset should work (schema exists from snapshot)
        let changeset_compressed = create_compressed_batch(vec![changeset]);
        let changeset_message = SignedMessage {
            sequence: 1,
            timestamp: 1700000001,
            message_type: MessageType::Changeset,
            payload: changeset_compressed,
            message_hash: "0x1".to_string(),
            signature: "0x0".to_string(),
            signer: "0x0".to_string(),
            cose_protected_header: vec![],
        };

        applier.apply_message(&changeset_message).unwrap();

        // Verify the change was applied
        let name: String = applier
            .conn
            .query_row("SELECT name FROM users WHERE id = 1", [], |row| row.get(0))
            .unwrap();
        assert_eq!(name, "Bob");
    }

    #[test]
    fn test_changeset_before_snapshot_fails_after_works() {
        // This test demonstrates the problem and solution:
        // 1. Changeset without schema → fails
        // 2. Snapshot provides schema → succeeds
        // 3. Same changeset (recreated) → now works

        // Create source database
        let source = Connection::open_in_memory().unwrap();
        source
            .execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)", [])
            .unwrap();
        source
            .execute("INSERT INTO users VALUES (1, 'Alice')", [])
            .unwrap();

        // Create changeset
        let changeset = create_test_changeset(&source);

        // Create empty target
        let mut applier = ChangesetApplier::in_memory().unwrap();

        // Step 1: Changeset fails (no schema)
        let compressed1 = create_compressed_batch(vec![changeset.clone()]);
        let message1 = SignedMessage {
            sequence: 0,
            timestamp: 1700000000,
            message_type: MessageType::Changeset,
            payload: compressed1,
            message_hash: "0x0".to_string(),
            signature: "0x0".to_string(),
            signer: "0x0".to_string(),
            cose_protected_header: vec![],
        };

        let result = applier.apply_message(&message1);
        assert!(result.is_err(), "Should fail without schema");

        // Step 2: Apply snapshot
        let snapshot_data = create_database_bytes_from(&source);
        let snapshot_compressed = create_compressed_snapshot(snapshot_data, 1);
        let snapshot_message = SignedMessage {
            sequence: 1,
            timestamp: 1700000001,
            message_type: MessageType::Snapshot,
            payload: snapshot_compressed,
            message_hash: "0x1".to_string(),
            signature: "0x0".to_string(),
            signer: "0x0".to_string(),
            cose_protected_header: vec![],
        };

        applier.apply_message(&snapshot_message).unwrap();

        // Note: The original changeset can't be re-applied because the snapshot
        // already includes the "Alice" data, and the changeset expects to
        // UPDATE from Alice to Bob. Since the snapshot includes Alice,
        // we can create a NEW changeset from this state.

        // Step 3: Create new changeset from current state and apply
        // (In practice, the snapshot would be at a point AFTER the DDL
        // but BEFORE the data changes, so changesets would apply cleanly)

        // For this test, we just verify the schema is now present
        let tables: Vec<String> = applier
            .conn
            .prepare(
                "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'",
            )
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();

        assert!(
            tables.contains(&"users".to_string()),
            "Schema should be restored by snapshot"
        );
    }

    #[test]
    fn test_sqlite_column_type_mismatch_behavior() {
        // This test documents SQLite's actual behavior with column type mismatches.
        // SQLite is dynamically typed - column type declarations are "type affinity"
        // hints, not strict constraints. This test verifies that changesets can
        // successfully apply even when column types differ.

        use rusqlite::session::ConflictAction;

        // === Case 1: TEXT changeset applied to INTEGER column ===
        // Create source with TEXT column
        let source = Connection::open_in_memory().unwrap();
        source
            .execute("CREATE TABLE t (id INTEGER PRIMARY KEY, val TEXT)", [])
            .unwrap();
        source
            .execute("INSERT INTO t VALUES (1, 'hello')", [])
            .unwrap();

        // Capture UPDATE changeset (TEXT value 'hello' -> 'world')
        let mut session = Session::new(&source).unwrap();
        session.attach(None::<&str>).unwrap();
        source
            .execute("UPDATE t SET val = 'world' WHERE id = 1", [])
            .unwrap();
        let mut changeset_text = Vec::new();
        session.changeset_strm(&mut changeset_text).unwrap();

        // Create target with INTEGER column (same name, different type)
        let target = Connection::open_in_memory().unwrap();
        target
            .execute("CREATE TABLE t (id INTEGER PRIMARY KEY, val INTEGER)", [])
            .unwrap();
        // Insert same data - SQLite stores 'hello' even in INTEGER column (dynamic typing)
        target
            .execute("INSERT INTO t VALUES (1, 'hello')", [])
            .unwrap();

        // Apply the TEXT changeset to INTEGER column
        let mut cursor = std::io::Cursor::new(&changeset_text);
        let result = target.apply_strm(&mut cursor, None::<fn(&str) -> bool>, |_, _| {
            ConflictAction::SQLITE_CHANGESET_ABORT
        });

        // SQLite succeeds because it's dynamically typed!
        assert!(
            result.is_ok(),
            "SQLite accepts TEXT changeset on INTEGER column due to dynamic typing"
        );

        // Verify the value was updated
        let val: String = target
            .query_row("SELECT val FROM t WHERE id = 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            val, "world",
            "Value should be updated even with type mismatch"
        );

        // === Case 2: INTEGER changeset applied to TEXT column ===
        let source2 = Connection::open_in_memory().unwrap();
        source2
            .execute(
                "CREATE TABLE nums (id INTEGER PRIMARY KEY, count INTEGER)",
                [],
            )
            .unwrap();
        source2
            .execute("INSERT INTO nums VALUES (1, 100)", [])
            .unwrap();

        let mut session2 = Session::new(&source2).unwrap();
        session2.attach(None::<&str>).unwrap();
        source2
            .execute("UPDATE nums SET count = 200 WHERE id = 1", [])
            .unwrap();
        let mut changeset_int = Vec::new();
        session2.changeset_strm(&mut changeset_int).unwrap();

        // Create target with TEXT column
        let target2 = Connection::open_in_memory().unwrap();
        target2
            .execute("CREATE TABLE nums (id INTEGER PRIMARY KEY, count TEXT)", [])
            .unwrap();
        target2
            .execute("INSERT INTO nums VALUES (1, 100)", [])
            .unwrap();

        // Apply the INTEGER changeset to TEXT column
        let mut cursor2 = std::io::Cursor::new(&changeset_int);
        let result2 = target2.apply_strm(&mut cursor2, None::<fn(&str) -> bool>, |_, _| {
            ConflictAction::SQLITE_CHANGESET_ABORT
        });

        assert!(
            result2.is_ok(),
            "SQLite accepts INTEGER changeset on TEXT column due to dynamic typing"
        );

        // Note: We read as String here because the column is declared as TEXT,
        // but SQLite stores the INTEGER value 200 correctly
        let count: String = target2
            .query_row("SELECT count FROM nums WHERE id = 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            count, "200",
            "Value should be updated even with type mismatch"
        );
    }

    #[test]
    fn test_column_type_info_available_from_schema() {
        // This test shows that we CAN get column type information from the target schema
        // using pragma_table_info. However, since SQLite is dynamically typed,
        // type mismatches don't cause changeset application to fail.

        let conn = Connection::open_in_memory().unwrap();
        conn.execute(
            "CREATE TABLE mixed (id INTEGER PRIMARY KEY, int_val INTEGER, text_val TEXT, real_val REAL)",
            [],
        )
        .unwrap();

        // Get column types from schema
        let mut stmt = conn
            .prepare("SELECT name, type FROM pragma_table_info('mixed')")
            .unwrap();
        let columns: Vec<(String, String)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();

        // Verify we can extract type information from schema
        assert_eq!(columns.len(), 4);
        assert!(columns.contains(&("id".to_string(), "INTEGER".to_string())));
        assert!(columns.contains(&("int_val".to_string(), "INTEGER".to_string())));
        assert!(columns.contains(&("text_val".to_string(), "TEXT".to_string())));
        assert!(columns.contains(&("real_val".to_string(), "REAL".to_string())));

        // Key insight: While we CAN get type info from schema, it doesn't matter
        // for changeset application because SQLite is dynamically typed.
        // Type mismatches are handled gracefully by SQLite itself.
    }
}
