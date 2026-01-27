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
        match message.message_type {
            MessageType::Changeset => self.apply_changeset_message(message),
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

    /// Apply a changeset message
    fn apply_changeset_message(&mut self, message: &SignedMessage) -> Result<()> {
        let batch: ChangesetBatchRequest = self.decompress_and_parse(message, "changeset batch")?;

        debug!(
            sequence = message.sequence,
            batch_id = %batch.batch_id,
            changeset_count = batch.changesets.len(),
            "Applying changeset batch"
        );

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
                std::env::temp_dir().join(format!("synddb_snapshot_restore_{id}.db"))
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

    /// Apply a single changeset to the database using the streaming API
    fn apply_single_changeset(conn: &Connection, data: &[u8]) -> Result<()> {
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

    /// Create an in-memory database and return its bytes
    fn create_test_database_bytes() -> Vec<u8> {
        use std::{
            fs,
            sync::atomic::{AtomicU64, Ordering},
        };

        // Use atomic counter for unique file names in parallel tests
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let thread_id = std::thread::current().id();

        // Create a file-based database with some data (so we can read the bytes)
        let temp_path =
            std::env::temp_dir().join(format!("test_snapshot_source_{id}_{thread_id:?}.db"));
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
        use std::fs;

        // Create a file-based applier
        let temp_path = std::env::temp_dir().join("test_snapshot_target.db");
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
}
