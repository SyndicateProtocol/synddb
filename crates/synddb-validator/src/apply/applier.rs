//! Changeset applier for reconstructing state from sequenced messages

use crate::error::ValidatorError;
use anyhow::{Context, Result};
use rusqlite::session::ConflictAction;
use rusqlite::Connection;
use std::io::{Cursor, Read};
use synddb_shared::types::{ChangesetBatchRequest, MessageType, SignedMessage};
use tracing::{debug, info, warn};

/// Applies changesets from sequenced messages to an `SQLite` database
pub struct ChangesetApplier {
    conn: Connection,
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

        Ok(Self { conn })
    }

    /// Create an applier with an in-memory database (for testing)
    pub fn in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().context("Failed to open in-memory database")?;

        Ok(Self { conn })
    }

    /// Get a reference to the underlying connection (for queries)
    pub const fn connection(&self) -> &Connection {
        &self.conn
    }

    /// Apply a signed message to the database
    ///
    /// Handles different message types:
    /// - `Changeset`: Decompress and apply changeset batch
    /// - `Snapshot`: Log only (snapshot restoration not implemented yet)
    /// - `Withdrawal`: Log only (no database changes)
    pub fn apply_message(&mut self, message: &SignedMessage) -> Result<()> {
        match message.message_type {
            MessageType::Changeset => self.apply_changeset_message(message),
            MessageType::Snapshot => {
                info!(
                    sequence = message.sequence,
                    "Snapshot message (restoration not yet implemented)"
                );
                Ok(())
            }
            MessageType::Withdrawal => {
                debug!(
                    sequence = message.sequence,
                    "Withdrawal message (no database changes)"
                );
                Ok(())
            }
        }
    }

    /// Apply a changeset message
    fn apply_changeset_message(&mut self, message: &SignedMessage) -> Result<()> {
        // 1. Decompress the payload
        let decompressed = Self::decompress(&message.payload).map_err(|e| {
            ValidatorError::DecompressionError(format!(
                "Failed to decompress sequence {}: {e}",
                message.sequence
            ))
        })?;

        // 2. Parse as ChangesetBatchRequest
        let batch: ChangesetBatchRequest = serde_json::from_slice(&decompressed).map_err(|e| {
            ValidatorError::ParseError(format!(
                "Failed to parse changeset batch at sequence {}: {e}",
                message.sequence
            ))
        })?;

        debug!(
            sequence = message.sequence,
            batch_id = %batch.batch_id,
            changeset_count = batch.changesets.len(),
            "Applying changeset batch"
        );

        // 3. Apply each changeset in a transaction
        let tx = self.conn.transaction().map_err(|e| {
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
            ValidatorError::DatabaseError(format!("Failed to commit transaction: {e}"))
        })?;

        info!(
            sequence = message.sequence,
            batch_id = %batch.batch_id,
            "Changeset batch applied successfully"
        );

        Ok(())
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

    /// Create a compressed changeset batch
    fn create_compressed_batch(changesets: Vec<Vec<u8>>) -> Vec<u8> {
        use std::io::Write;

        let batch = ChangesetBatchRequest {
            batch_id: "test-batch".to_string(),
            changesets: changesets
                .into_iter()
                .enumerate()
                .map(|(i, data)| synddb_shared::types::ChangesetData {
                    data,
                    sequence: i as u64,
                    timestamp: 1700000000,
                })
                .collect(),
            attestation_token: None,
        };

        let json = serde_json::to_vec(&batch).unwrap();

        // Compress with zstd
        let mut encoder = zstd::Encoder::new(Vec::new(), 3).unwrap();
        encoder.write_all(&json).unwrap();
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
            .connection()
            .execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)", [])
            .unwrap();
        applier
            .connection()
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
        };

        // Apply the changeset
        applier.apply_message(&message).unwrap();

        // Verify the change was applied
        let name: String = applier
            .connection()
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
        };

        // Should succeed with no changes
        assert!(applier.apply_message(&message).is_ok());
    }

    #[test]
    fn test_snapshot_message_no_change() {
        let mut applier = ChangesetApplier::in_memory().unwrap();

        let message = SignedMessage {
            sequence: 0,
            timestamp: 1700000000,
            message_type: MessageType::Snapshot,
            payload: vec![],
            message_hash: "0x0".to_string(),
            signature: "0x0".to_string(),
            signer: "0x0".to_string(),
        };

        // Should succeed (logging only for now)
        assert!(applier.apply_message(&message).is_ok());
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
            .connection()
            .execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)", [])
            .unwrap();
        applier
            .connection()
            .execute("INSERT INTO users VALUES (1, 'Alice')", [])
            .unwrap();
        applier
            .connection()
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
        };

        // Apply the batch
        applier.apply_message(&message).unwrap();

        // Verify both changes were applied
        let name1: String = applier
            .connection()
            .query_row("SELECT name FROM users WHERE id = 1", [], |row| row.get(0))
            .unwrap();
        let name2: String = applier
            .connection()
            .query_row("SELECT name FROM users WHERE id = 2", [], |row| row.get(0))
            .unwrap();

        assert_eq!(name1, "Bob");
        assert_eq!(name2, "Dave");
    }
}
