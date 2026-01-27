//! E2E test harness for `SyndDB` pipeline
//!
//! Provides in-process testing of the full pipeline:
//! source DB -> changeset capture -> sequencer -> validator -> replica DB

use alloy::signers::local::PrivateKeySigner;
use anyhow::{Context, Result};
use rusqlite::{session::Session, Connection};
use std::sync::Arc;
use synddb_sequencer::{inbox::Inbox, signer::MessageSigner};
use synddb_shared::types::{
    cbor::{message::CborMessageType, verify::verifying_key_from_bytes},
    message::SignedMessage,
    payloads::{ChangesetBatchRequest, ChangesetData, SnapshotData, SnapshotRequest},
};
use synddb_validator::{
    sync::{fetcher::StorageFetcher, providers::mock::MockFetcher},
    validator::Validator,
};
use tokio::sync::watch;
use tracing::debug;

/// Test private key (Anvil default account #0)
/// DO NOT use in production!
const TEST_PRIVATE_KEY: &str = "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

/// E2E test harness that coordinates source DB, sequencer, and validator
pub struct E2EHarness {
    /// Source database (application side)
    source_conn: Connection,
    /// Sequencer inbox (assigns sequences, signs)
    inbox: Inbox,
    /// Messages storage (in-memory, shared with validator)
    fetcher: Arc<MockFetcher>,
    /// Validator with replica database
    validator: Validator,
    /// Sequencer public key (64 bytes, uncompressed, no prefix)
    pubkey: [u8; 64],
    /// Private key signer for creating test messages
    signer: PrivateKeySigner,
    /// Shutdown sender for validator
    _shutdown_tx: watch::Sender<bool>,
}

impl std::fmt::Debug for E2EHarness {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("E2EHarness")
            .field("current_sequence", &self.inbox.current_sequence())
            .field("pubkey", &hex::encode(&self.pubkey[..8]))
            .finish()
    }
}

impl E2EHarness {
    /// Create a new E2E harness with in-memory databases
    pub fn new() -> Result<Self> {
        let source_conn = Connection::open_in_memory().context("Failed to open source database")?;

        // Setup sequencer
        let message_signer =
            MessageSigner::new(TEST_PRIVATE_KEY).context("Failed to create message signer")?;
        let pubkey = message_signer.public_key();
        let inbox = Inbox::new(message_signer);

        // Setup mock fetcher and validator
        let fetcher: Arc<MockFetcher> = Arc::new(MockFetcher::new());
        let fetcher_dyn: Arc<dyn StorageFetcher> = Arc::clone(&fetcher) as Arc<dyn StorageFetcher>;
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let validator = Validator::in_memory(fetcher_dyn, pubkey, shutdown_rx)
            .context("Failed to create validator")?;

        // Parse signer for test message creation
        let signer: PrivateKeySigner = TEST_PRIVATE_KEY
            .parse()
            .context("Failed to parse private key")?;

        Ok(Self {
            source_conn,
            inbox,
            fetcher,
            validator,
            pubkey,
            signer,
            _shutdown_tx: shutdown_tx,
        })
    }

    /// Get reference to source database connection
    pub const fn source(&self) -> &Connection {
        &self.source_conn
    }

    /// Get reference to validator's replica database connection
    pub const fn replica(&self) -> &Connection {
        self.validator.connection()
    }

    /// Get current sequence number (next to be assigned)
    pub fn current_sequence(&self) -> u64 {
        self.inbox.current_sequence()
    }

    /// Get the sequencer's public key
    pub const fn sequencer_pubkey(&self) -> &[u8; 64] {
        &self.pubkey
    }

    /// Get all messages that have been sequenced
    ///
    /// Note: This creates a new runtime, so it should not be called from within
    /// an async context. For async contexts, use `messages_async()`.
    pub fn messages(&self) -> Vec<SignedMessage> {
        // Try to use existing runtime handle, or create a new one
        tokio::runtime::Handle::try_current().map_or_else(
            |_| {
                // Not in async context, create our own runtime
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("Failed to create runtime");
                let mut messages = Vec::new();
                for seq in 0..self.inbox.current_sequence() {
                    if let Ok(Some(msg)) = rt.block_on(self.fetcher.get(seq)) {
                        messages.push(msg);
                    }
                }
                messages
            },
            |_handle| {
                // We're in an async context, use spawn_blocking
                let fetcher = Arc::clone(&self.fetcher);
                let current_seq = self.inbox.current_sequence();
                std::thread::scope(|s| {
                    s.spawn(|| {
                        let rt = tokio::runtime::Builder::new_current_thread()
                            .enable_all()
                            .build()
                            .expect("Failed to create runtime");
                        let mut messages = Vec::new();
                        for seq in 0..current_seq {
                            if let Ok(Some(msg)) = rt.block_on(fetcher.get(seq)) {
                                messages.push(msg);
                            }
                        }
                        messages
                    })
                    .join()
                    .expect("Thread panicked")
                })
            },
        )
    }

    /// Execute DDL on source database (schema changes)
    ///
    /// Note: This doesn't capture changesets since DDL requires snapshots.
    pub fn execute_ddl(&self, sql: &str) -> Result<()> {
        self.source_conn
            .execute_batch(sql)
            .context("Failed to execute DDL on source")?;
        Ok(())
    }

    /// Execute DML on source database and capture the changeset
    ///
    /// Returns the raw changeset bytes (not sequenced yet).
    pub fn execute_dml(&self, sql: &str) -> Result<Vec<u8>> {
        // Create session to capture changes
        let mut session = Session::new(&self.source_conn).context("Failed to create session")?;
        session
            .attach(None::<&str>)
            .context("Failed to attach session")?;

        // Execute the SQL
        self.source_conn
            .execute_batch(sql)
            .context("Failed to execute DML on source")?;

        // Extract changeset
        let mut output = Vec::new();
        session
            .changeset_strm(&mut output)
            .context("Failed to extract changeset")?;

        Ok(output)
    }

    /// Execute DML, capture changeset, sequence it, and add to fetcher
    ///
    /// Returns the assigned sequence number.
    pub fn execute_and_sequence(&mut self, sql: &str) -> Result<u64> {
        let changeset = self.execute_dml(sql)?;
        self.sequence_changeset(changeset)
    }

    /// Sequence a changeset and add to the fetcher
    ///
    /// Returns the assigned sequence number.
    pub fn sequence_changeset(&mut self, changeset: Vec<u8>) -> Result<u64> {
        let sequence = self.inbox.current_sequence();
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("Time went backwards")
            .as_secs();

        // Create batch payload (same format as synddb-client)
        let batch = ChangesetBatchRequest {
            batch_id: format!("test-batch-{sequence}"),
            changesets: vec![ChangesetData {
                data: changeset,
                sequence,
                timestamp,
            }],
            attestation_token: None,
        };

        // Serialize to CBOR
        let mut cbor = Vec::new();
        ciborium::into_writer(&batch, &mut cbor).context("Failed to serialize batch to CBOR")?;

        // Sequence and sign via inbox
        let (cbor_msg, receipt) = self
            .inbox
            .sequence_message(CborMessageType::Changeset, cbor)
            .context("Failed to sequence message")?;

        // Convert to SignedMessage and add to fetcher
        let pubkey =
            verifying_key_from_bytes(&self.pubkey).context("Failed to parse public key")?;
        let signed_msg = cbor_msg
            .to_signed_message(&pubkey)
            .context("Failed to convert to SignedMessage")?;

        self.fetcher.add_message(signed_msg);

        debug!(sequence = receipt.sequence, "Sequenced changeset");
        Ok(receipt.sequence)
    }

    /// Create and sequence a snapshot of the current source database
    ///
    /// Returns the assigned sequence number.
    pub fn create_and_sequence_snapshot(&mut self) -> Result<u64> {
        let sequence = self.inbox.current_sequence();
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("Time went backwards")
            .as_secs();

        // Create snapshot by writing to a temp file
        let temp_path = tempfile::NamedTempFile::new().context("Failed to create temp file")?;
        self.source_conn
            .backup(rusqlite::DatabaseName::Main, temp_path.path(), None)
            .context("Failed to backup to temp file")?;
        let snapshot_bytes =
            std::fs::read(temp_path.path()).context("Failed to read snapshot file")?;

        // Create snapshot request
        let request = SnapshotRequest {
            message_id: format!("snapshot-{sequence}"),
            snapshot: SnapshotData {
                data: snapshot_bytes,
                sequence,
                timestamp,
            },
            attestation_token: None,
        };

        // Serialize to CBOR
        let mut cbor = Vec::new();
        ciborium::into_writer(&request, &mut cbor)
            .context("Failed to serialize snapshot to CBOR")?;

        // Sequence and sign via inbox
        let (cbor_msg, receipt) = self
            .inbox
            .sequence_message(CborMessageType::Snapshot, cbor)
            .context("Failed to sequence snapshot")?;

        // Convert to SignedMessage and add to fetcher
        let pubkey =
            verifying_key_from_bytes(&self.pubkey).context("Failed to parse public key")?;
        let signed_msg = cbor_msg
            .to_signed_message(&pubkey)
            .context("Failed to convert to SignedMessage")?;

        self.fetcher.add_message(signed_msg);

        debug!(sequence = receipt.sequence, "Sequenced snapshot");
        Ok(receipt.sequence)
    }

    /// Sync validator to head (process all available messages)
    ///
    /// Returns the number of messages synced.
    pub async fn sync_validator(&mut self) -> Result<u64> {
        self.validator.sync_to_head().await
    }

    /// Sync validator to a specific sequence number
    pub async fn sync_validator_to(&mut self, sequence: u64) -> Result<bool> {
        self.validator.sync_one(sequence).await
    }

    /// Get validator's last synced sequence
    pub fn validator_last_sequence(&self) -> Result<Option<u64>> {
        self.validator.last_sequence()
    }

    /// Get count of pending changesets in validator's audit trail
    pub fn pending_changeset_count(&self) -> Result<u64> {
        self.validator.pending_changeset_count()
    }

    /// Snapshot the source database state for comparison
    pub fn snapshot_source_state(&self) -> Result<DatabaseSnapshot> {
        DatabaseSnapshot::from_connection(&self.source_conn)
    }

    /// Snapshot the replica database state for comparison
    pub fn snapshot_replica_state(&self) -> Result<DatabaseSnapshot> {
        DatabaseSnapshot::from_connection(self.replica())
    }

    /// Verify that source and replica have identical state
    pub fn verify_consistency(&self) -> Result<()> {
        let source = self.snapshot_source_state()?;
        let replica = self.snapshot_replica_state()?;

        if source != replica {
            anyhow::bail!(
                "State mismatch!\nSource tables: {:?}\nReplica tables: {:?}",
                source.tables.keys().collect::<Vec<_>>(),
                replica.tables.keys().collect::<Vec<_>>()
            );
        }

        Ok(())
    }

    /// Setup identical schema on both source and replica
    pub fn setup_schema(&self, sql: &str) -> Result<()> {
        self.source_conn
            .execute_batch(sql)
            .context("Failed to setup schema on source")?;
        self.replica()
            .execute_batch(sql)
            .context("Failed to setup schema on replica")?;
        Ok(())
    }

    /// Get access to the `MockFetcher` for fault injection
    pub fn fetcher(&self) -> &MockFetcher {
        &self.fetcher
    }
}

/// Snapshot of database state for comparison
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatabaseSnapshot {
    /// Table name -> rows (as JSON-like strings for comparison)
    pub tables: std::collections::BTreeMap<String, Vec<String>>,
}

impl DatabaseSnapshot {
    /// Create a snapshot from a database connection
    pub fn from_connection(conn: &Connection) -> Result<Self> {
        let mut tables = std::collections::BTreeMap::new();

        // Get all user tables
        let mut stmt = conn.prepare(
            "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'",
        )?;
        let table_names: Vec<String> = stmt
            .query_map([], |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()?;

        for table_name in table_names {
            let mut rows = Vec::new();

            // Get all rows from this table, ordered for consistent comparison
            let query = format!(
                "SELECT * FROM \"{}\" ORDER BY rowid",
                table_name.replace('"', "\"\"")
            );

            let mut stmt = conn.prepare(&query)?;
            let column_count = stmt.column_count();

            let row_iter = stmt.query_map([], |row| {
                let mut values = Vec::new();
                for i in 0..column_count {
                    let value: rusqlite::types::Value = row.get(i)?;
                    values.push(format!("{value:?}"));
                }
                Ok(values.join(", "))
            })?;

            for row in row_iter {
                rows.push(row?);
            }

            tables.insert(table_name, rows);
        }

        Ok(Self { tables })
    }

    /// Check if this snapshot equals another
    pub fn is_equivalent(&self, other: &Self) -> bool {
        self == other
    }

    /// Get the diff between two snapshots
    pub fn diff(&self, other: &Self) -> String {
        let mut diffs = Vec::new();

        // Check for tables in self but not in other
        for table in self.tables.keys() {
            if !other.tables.contains_key(table) {
                diffs.push(format!("Table '{}' missing in other", table));
            }
        }

        // Check for tables in other but not in self
        for table in other.tables.keys() {
            if !self.tables.contains_key(table) {
                diffs.push(format!("Table '{}' missing in self", table));
            }
        }

        // Check for row differences in common tables
        for (table, rows) in &self.tables {
            if let Some(other_rows) = other.tables.get(table) {
                if rows != other_rows {
                    diffs.push(format!(
                        "Table '{}': {} rows vs {} rows",
                        table,
                        rows.len(),
                        other_rows.len()
                    ));
                }
            }
        }

        diffs.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_harness_creation() {
        let harness = E2EHarness::new().expect("Failed to create harness");
        assert_eq!(harness.current_sequence(), 0);
    }

    #[test]
    fn test_schema_setup() {
        let harness = E2EHarness::new().expect("Failed to create harness");
        harness
            .setup_schema("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)")
            .expect("Failed to setup schema");

        // Verify schema on source
        let source_count: i64 = harness
            .source()
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name = 'users'",
                [],
                |row| row.get(0),
            )
            .expect("Failed to query source");
        assert_eq!(source_count, 1);

        // Verify schema on replica
        let replica_count: i64 = harness
            .replica()
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name = 'users'",
                [],
                |row| row.get(0),
            )
            .expect("Failed to query replica");
        assert_eq!(replica_count, 1);
    }

    #[test]
    fn test_execute_dml_captures_changeset() {
        let harness = E2EHarness::new().expect("Failed to create harness");
        harness
            .setup_schema("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)")
            .expect("Failed to setup schema");

        // Insert initial data for changeset capture
        harness
            .source()
            .execute("INSERT INTO users VALUES (1, 'Alice')", [])
            .expect("Failed to insert initial data");
        harness
            .replica()
            .execute("INSERT INTO users VALUES (1, 'Alice')", [])
            .expect("Failed to insert initial data on replica");

        // Execute DML and capture changeset
        let changeset = harness
            .execute_dml("UPDATE users SET name = 'Bob' WHERE id = 1")
            .expect("Failed to execute DML");

        // Changeset should not be empty
        assert!(!changeset.is_empty(), "Changeset should not be empty");
    }

    #[tokio::test]
    async fn test_full_roundtrip() {
        let mut harness = E2EHarness::new().expect("Failed to create harness");
        harness
            .setup_schema("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)")
            .expect("Failed to setup schema");

        // Insert initial data on both
        harness
            .source()
            .execute("INSERT INTO users VALUES (1, 'Alice')", [])
            .expect("Failed to insert on source");
        harness
            .replica()
            .execute("INSERT INTO users VALUES (1, 'Alice')", [])
            .expect("Failed to insert on replica");

        // Execute, sequence, and sync
        let seq = harness
            .execute_and_sequence("UPDATE users SET name = 'Bob' WHERE id = 1")
            .expect("Failed to execute and sequence");
        assert_eq!(seq, 0);

        // Sync validator
        let synced = harness
            .sync_validator()
            .await
            .expect("Failed to sync validator");
        assert_eq!(synced, 1);

        // Verify consistency
        harness
            .verify_consistency()
            .expect("State should be consistent");

        // Verify the actual value
        let name: String = harness
            .replica()
            .query_row("SELECT name FROM users WHERE id = 1", [], |row| row.get(0))
            .expect("Failed to query replica");
        assert_eq!(name, "Bob");
    }

    #[tokio::test]
    async fn test_multiple_operations() {
        let mut harness = E2EHarness::new().expect("Failed to create harness");
        harness
            .setup_schema("CREATE TABLE counter (id INTEGER PRIMARY KEY, value INTEGER)")
            .expect("Failed to setup schema");

        // Insert initial value on both
        harness
            .source()
            .execute("INSERT INTO counter VALUES (1, 0)", [])
            .expect("Failed to insert on source");
        harness
            .replica()
            .execute("INSERT INTO counter VALUES (1, 0)", [])
            .expect("Failed to insert on replica");

        // Perform multiple updates
        for i in 1..=5 {
            let sql = format!("UPDATE counter SET value = {} WHERE id = 1", i);
            harness
                .execute_and_sequence(&sql)
                .expect("Failed to execute and sequence");
        }

        // Verify sequence counter
        assert_eq!(harness.current_sequence(), 5);

        // Sync all
        let synced = harness
            .sync_validator()
            .await
            .expect("Failed to sync validator");
        assert_eq!(synced, 5);

        // Verify final value
        let value: i64 = harness
            .replica()
            .query_row("SELECT value FROM counter WHERE id = 1", [], |row| {
                row.get(0)
            })
            .expect("Failed to query replica");
        assert_eq!(value, 5);
    }

    #[test]
    fn test_database_snapshot() {
        let conn = Connection::open_in_memory().expect("Failed to open connection");
        conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, data TEXT)", [])
            .expect("Failed to create table");
        conn.execute("INSERT INTO test VALUES (1, 'hello')", [])
            .expect("Failed to insert");
        conn.execute("INSERT INTO test VALUES (2, 'world')", [])
            .expect("Failed to insert");

        let snapshot = DatabaseSnapshot::from_connection(&conn).expect("Failed to create snapshot");

        assert!(snapshot.tables.contains_key("test"));
        assert_eq!(snapshot.tables["test"].len(), 2);
    }

    #[test]
    fn test_snapshot_comparison() {
        let conn1 = Connection::open_in_memory().expect("Failed to open connection");
        let conn2 = Connection::open_in_memory().expect("Failed to open connection");

        for conn in [&conn1, &conn2] {
            conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, data TEXT)", [])
                .expect("Failed to create table");
            conn.execute("INSERT INTO test VALUES (1, 'hello')", [])
                .expect("Failed to insert");
        }

        let snap1 = DatabaseSnapshot::from_connection(&conn1).expect("Failed to snapshot");
        let snap2 = DatabaseSnapshot::from_connection(&conn2).expect("Failed to snapshot");

        assert!(snap1.is_equivalent(&snap2));
    }
}
