//! E2E tests for real `SQLite` snapshot flow
//!
//! Tests the full snapshot lifecycle:
//! 1. Create a real `SQLite` database with known state
//! 2. Capture it as a snapshot (raw .db bytes)
//! 3. Send through sequencer
//! 4. Verify storage and validator processing

use anyhow::{ensure, Context};
use rusqlite::{session::Session, Connection};
use std::io::Write;
use tracing::info;

use crate::{
    result::{TestCase, TestCaseResult},
    runner::TestRunner,
};

/// Create an ` SQLite ` database with known test data and return its bytes
fn create_test_database() -> anyhow::Result<(Vec<u8>, TestDatabaseState)> {
    // Create an in-memory database
    let conn = Connection::open_in_memory()?;

    // Create schema
    conn.execute_batch(
        "
        CREATE TABLE users (id INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            email TEXT NOT NULL);
        CREATE TABLE orders (id INTEGER PRIMARY KEY,
            user_id INTEGER NOT NULL,
            amount INTEGER NOT NULL,
            status TEXT NOT NULL);
        ",
    )?;

    // Insert known test data
    conn.execute(
        "INSERT INTO users (id, name, email) VALUES (1, 'Alice', 'alice@example.com')",
        [],
    )?;
    conn.execute(
        "INSERT INTO users (id, name, email) VALUES (2, 'Bob', 'bob@example.com')",
        [],
    )?;
    conn.execute(
        "INSERT INTO orders (id, user_id, amount, status) VALUES (100, 1, 5000, 'completed')",
        [],
    )?;
    conn.execute(
        "INSERT INTO orders (id, user_id, amount, status) VALUES (101, 2, 3000, 'pending')",
        [],
    )?;

    // Query the state for verification
    let user_count: i64 = conn.query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0))?;
    let order_count: i64 = conn.query_row("SELECT COUNT(*) FROM orders", [], |r| r.get(0))?;
    let total_amount: i64 = conn.query_row("SELECT SUM(amount) FROM orders", [], |r| r.get(0))?;

    let state = TestDatabaseState {
        user_count: user_count as usize,
        order_count: order_count as usize,
        total_order_amount: total_amount,
    };

    // Export database to bytes using backup API
    let db_bytes = export_database_to_bytes(&conn)?;

    Ok((db_bytes, state))
}

/// Export an in-memory database to bytes
fn export_database_to_bytes(conn: &Connection) -> anyhow::Result<Vec<u8>> {
    // Create a temporary file to backup to
    let temp_path =
        std::env::temp_dir().join(format!("synddb_e2e_snapshot_{}.db", std::process::id()));

    // Backup to file
    {
        let mut dest = Connection::open(&temp_path)?;
        let backup = rusqlite::backup::Backup::new(conn, &mut dest)?;
        backup.run_to_completion(100, std::time::Duration::from_millis(10), None)?;
    }

    // Read the file bytes
    let bytes = std::fs::read(&temp_path)?;

    // Clean up
    let _ = std::fs::remove_file(&temp_path);

    Ok(bytes)
}

/// Known state of the test database for verification
#[derive(Debug, Clone)]
struct TestDatabaseState {
    user_count: usize,
    order_count: usize,
    total_order_amount: i64,
}

/// Compress snapshot data as the sequencer expects (zstd compressed JSON with snapshot field)
fn compress_snapshot_payload(
    message_id: &str,
    db_bytes: &[u8],
    sequence: u64,
    timestamp: u64,
) -> anyhow::Result<Vec<u8>> {
    let payload = serde_json::json!({
        "message_id": message_id,
        "snapshot": {
            "data": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, db_bytes),
            "timestamp": timestamp,
            "sequence": sequence
        }
    });

    let json_bytes = serde_json::to_vec(&payload)?;

    // Compress with zstd
    let mut encoder = zstd::Encoder::new(Vec::new(), 3)?;
    encoder.write_all(&json_bytes)?;
    Ok(encoder.finish()?)
}

impl TestRunner {
    /// Test that a real `SQLite` database snapshot can be sent and stored
    pub(crate) async fn test_real_sqlite_snapshot(&self) -> TestCaseResult {
        TestCase::new(
            "real_sqlite_snapshot",
            "Real SQLite database snapshot is sequenced and stored",
        )
        .run(|| async {
            // Create a real SQLite database with known state
            let (db_bytes, state) = create_test_database()?;

            info!(
                db_size = db_bytes.len(),
                user_count = state.user_count,
                order_count = state.order_count,
                total_amount = state.total_order_amount,
                "Created test database"
            );

            // Verify it's a valid SQLite database (starts with magic header)
            ensure!(
                db_bytes.len() >= 16 && &db_bytes[0..16] == b"SQLite format 3\x00",
                "Database bytes don't have valid SQLite header"
            );

            // Get current sequence before sending snapshot
            let status_before = self.sequencer.status().await?;
            let seq_before = status_before.current_sequence;

            // Send the real database as a snapshot
            let response = self
                .sequencer
                .send_snapshot("e2e-real-sqlite-snapshot", &db_bytes, 1)
                .await
                .context("Failed to send snapshot to sequencer")?;

            info!(
                sequence = response.sequence,
                message_hash = %response.message_hash,
                db_size = db_bytes.len(),
                "Real SQLite snapshot sequenced"
            );

            // Verify sequence was assigned (should be >= seq_before since seq_before is the
            // current sequence at time of query, and our snapshot gets the next one)
            ensure!(
                response.sequence >= seq_before,
                "Snapshot sequence {} should be >= previous sequence {}",
                response.sequence,
                seq_before
            );

            // Wait for snapshot to appear in storage
            let mut found_in_storage = false;
            for _ in 0..30 {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                if let Some(msg) = self
                    .sequencer
                    .try_fetch_storage_message(response.sequence)
                    .await?
                {
                    // Verify it's a snapshot type
                    ensure!(
                        msg.message_type == synddb_shared::types::message::MessageType::Snapshot,
                        "Expected Snapshot message type, got {:?}",
                        msg.message_type
                    );

                    // Verify sequence matches
                    ensure!(
                        msg.sequence == response.sequence,
                        "Sequence mismatch in storage: expected {}, got {}",
                        response.sequence,
                        msg.sequence
                    );

                    found_in_storage = true;
                    info!(
                        sequence = msg.sequence,
                        payload_size = msg.payload.len(),
                        "Snapshot found in storage"
                    );
                    break;
                }
            }

            ensure!(
                found_in_storage,
                "Snapshot did not appear in storage within 3s"
            );

            Ok(())
        })
        .await
    }

    /// Test that validator processes the real `SQLite` snapshot
    pub(crate) async fn test_validator_processes_sqlite_snapshot(&self) -> TestCaseResult {
        TestCase::new(
            "validator_processes_sqlite_snapshot",
            "Validator syncs past real SQLite snapshot",
        )
        .run(|| async {
            // Create and send a snapshot
            let (db_bytes, state) = create_test_database()?;

            info!(
                db_size = db_bytes.len(),
                user_count = state.user_count,
                order_count = state.order_count,
                "Sending real SQLite snapshot for validator test"
            );

            let response = self
                .sequencer
                .send_snapshot("e2e-validator-sqlite-snapshot", &db_bytes, 2)
                .await
                .context("Failed to send snapshot")?;

            let snapshot_sequence = response.sequence;
            info!(
                sequence = snapshot_sequence,
                "Snapshot sequenced, waiting for validator to process"
            );

            // Wait for validator to sync past the snapshot sequence
            let mut validator_synced = false;
            for _ in 0..60 {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;

                let validator_status = self.validator.status().await?;

                if let Some(last_seq) = validator_status.last_sequence {
                    if last_seq >= snapshot_sequence {
                        info!(
                            validator_sequence = last_seq,
                            snapshot_sequence = snapshot_sequence,
                            "Validator synced past snapshot"
                        );
                        validator_synced = true;
                        break;
                    }
                    info!(
                        validator_sequence = last_seq,
                        snapshot_sequence = snapshot_sequence,
                        "Waiting for validator to reach snapshot..."
                    );
                }
            }

            ensure!(
                validator_synced,
                "Validator did not sync past snapshot sequence {} within 30s",
                snapshot_sequence
            );

            // Also check validator2
            let mut validator2_synced = false;
            for _ in 0..30 {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;

                let validator2_status = self.validator2.status().await?;

                if let Some(last_seq) = validator2_status.last_sequence {
                    if last_seq >= snapshot_sequence {
                        info!(
                            validator2_sequence = last_seq,
                            snapshot_sequence = snapshot_sequence,
                            "Validator2 also synced past snapshot"
                        );
                        validator2_synced = true;
                        break;
                    }
                }
            }

            ensure!(
                validator2_synced,
                "Validator2 did not sync past snapshot sequence {} within 15s",
                snapshot_sequence
            );

            Ok(())
        })
        .await
    }

    /// Test snapshot with larger database (stress test)
    pub(crate) async fn test_large_sqlite_snapshot(&self) -> TestCaseResult {
        TestCase::new(
            "large_sqlite_snapshot",
            "Large SQLite database snapshot (~100KB) is handled correctly",
        )
        .run(|| async {
            // Create a larger database
            let conn = Connection::open_in_memory()?;

            conn.execute_batch(
                "
                CREATE TABLE data (
                    id INTEGER PRIMARY KEY,
                    key TEXT NOT NULL,
                    value BLOB NOT NULL
                );
                ",
            )?;

            // Insert enough data to create a ~100KB database
            let mut stmt = conn.prepare("INSERT INTO data (key, value) VALUES (?, ?)")?;
            for i in 0..1000 {
                let key = format!("key_{:05}", i);
                let value = vec![0xABu8; 100]; // 100 bytes per row
                stmt.execute(rusqlite::params![key, value])?;
            }
            drop(stmt);

            let row_count: i64 = conn.query_row("SELECT COUNT(*) FROM data", [], |r| r.get(0))?;

            // Export to bytes
            let db_bytes = export_database_to_bytes(&conn)?;

            info!(
                db_size = db_bytes.len(),
                row_count = row_count,
                "Created large test database"
            );

            ensure!(
                db_bytes.len() > 50_000,
                "Database should be > 50KB, got {} bytes",
                db_bytes.len()
            );

            // Send as snapshot
            let response = self
                .sequencer
                .send_snapshot("e2e-large-sqlite-snapshot", &db_bytes, 3)
                .await
                .context("Failed to send large snapshot")?;

            info!(
                sequence = response.sequence,
                db_size = db_bytes.len(),
                "Large snapshot sequenced"
            );

            // Wait for it to appear in storage
            let mut found = false;
            for _ in 0..30 {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                if self
                    .sequencer
                    .try_fetch_storage_message(response.sequence)
                    .await?
                    .is_some()
                {
                    found = true;
                    break;
                }
            }

            ensure!(found, "Large snapshot did not appear in storage within 3s");

            info!(
                sequence = response.sequence,
                db_size = db_bytes.len(),
                "Large snapshot verified in storage"
            );

            Ok(())
        })
        .await
    }

    /// Test snapshot followed by changeset application
    ///
    /// This tests the full restore flow:
    /// 1. Create database with initial state
    /// 2. Send as snapshot
    /// 3. Make changes and capture as changesets
    /// 4. Send changesets
    /// 5. Verify validators process both snapshot and changesets
    pub(crate) async fn test_snapshot_then_changesets(&self) -> TestCaseResult {
        TestCase::new(
            "snapshot_then_changesets",
            "Snapshot followed by changesets is processed correctly",
        )
        .run(|| async {
            // Create initial database state
            let conn = Connection::open_in_memory()?;
            conn.execute_batch(
                "
                CREATE TABLE accounts (
                    id INTEGER PRIMARY KEY,
                    name TEXT NOT NULL,
                    balance INTEGER NOT NULL DEFAULT 0
                );
                INSERT INTO accounts (id, name, balance) VALUES (1, 'Alice', 1000);
                INSERT INTO accounts (id, name, balance) VALUES (2, 'Bob', 500);
                ",
            )?;

            // Export initial state as snapshot
            let snapshot_bytes = export_database_to_bytes(&conn)?;

            info!(
                snapshot_size = snapshot_bytes.len(),
                "Created initial snapshot with 2 accounts"
            );

            // Send snapshot
            let snapshot_response = self
                .sequencer
                .send_snapshot("e2e-snapshot-changeset-test", &snapshot_bytes, 100)
                .await
                .context("Failed to send initial snapshot")?;

            let snapshot_sequence = snapshot_response.sequence;
            info!(sequence = snapshot_sequence, "Initial snapshot sequenced");

            // Now generate changesets that modify the database
            // Changeset 1: Transfer 100 from Alice to Bob
            let changeset1 = {
                let mut session = Session::new(&conn)?;
                session.attach(None::<&str>)?;

                conn.execute("UPDATE accounts SET balance = 900 WHERE id = 1", [])?;
                conn.execute("UPDATE accounts SET balance = 600 WHERE id = 2", [])?;

                let mut changeset = Vec::new();
                session.changeset_strm(&mut changeset)?;
                changeset
            };

            info!(
                changeset_size = changeset1.len(),
                "Generated changeset 1: transfer 100 from Alice to Bob"
            );

            // Send changeset 1
            let cs1_response = self
                .sequencer
                .send_changeset("e2e-changeset-batch-1", &changeset1, 101)
                .await
                .context("Failed to send changeset 1")?;

            info!(sequence = cs1_response.sequence, "Changeset 1 sequenced");

            // Changeset 2: Add a new account
            let changeset2 = {
                let mut session = Session::new(&conn)?;
                session.attach(None::<&str>)?;

                conn.execute(
                    "INSERT INTO accounts (id, name, balance) VALUES (3, 'Charlie', 250)",
                    [],
                )?;

                let mut changeset = Vec::new();
                session.changeset_strm(&mut changeset)?;
                changeset
            };

            info!(
                changeset_size = changeset2.len(),
                "Generated changeset 2: add Charlie account"
            );

            // Send changeset 2
            let cs2_response = self
                .sequencer
                .send_changeset("e2e-changeset-batch-2", &changeset2, 102)
                .await
                .context("Failed to send changeset 2")?;

            let final_sequence = cs2_response.sequence;
            info!(sequence = final_sequence, "Changeset 2 sequenced");

            // Wait for all messages to appear in storage
            info!("Waiting for messages to appear in storage...");
            let mut all_found = false;
            for _ in 0..30 {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;

                let snapshot_msg = self
                    .sequencer
                    .try_fetch_storage_message(snapshot_sequence)
                    .await?;
                let cs1_msg = self
                    .sequencer
                    .try_fetch_storage_message(cs1_response.sequence)
                    .await?;
                let cs2_msg = self
                    .sequencer
                    .try_fetch_storage_message(final_sequence)
                    .await?;

                if let (Some(snapshot_msg), Some(cs1_msg), Some(cs2_msg)) =
                    (snapshot_msg, cs1_msg, cs2_msg)
                {
                    // Verify message types
                    ensure!(
                        snapshot_msg.message_type
                            == synddb_shared::types::message::MessageType::Snapshot,
                        "Expected snapshot message type, got {:?}",
                        snapshot_msg.message_type
                    );
                    ensure!(
                        cs1_msg.message_type
                            == synddb_shared::types::message::MessageType::Changeset,
                        "Expected changeset message type for cs1, got {:?}",
                        cs1_msg.message_type
                    );
                    ensure!(
                        cs2_msg.message_type
                            == synddb_shared::types::message::MessageType::Changeset,
                        "Expected changeset message type for cs2, got {:?}",
                        cs2_msg.message_type
                    );

                    all_found = true;
                    info!(
                        snapshot_seq = snapshot_sequence,
                        cs1_seq = cs1_response.sequence,
                        cs2_seq = final_sequence,
                        "All messages found in storage with correct types"
                    );
                    break;
                }
            }

            ensure!(all_found, "Not all messages appeared in storage within 3s");

            // Wait for validators to sync past all messages
            info!("Waiting for validators to sync...");
            let mut validator_synced = false;
            for _ in 0..60 {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;

                let v1_status = self.validator.status().await?;
                let v2_status = self.validator2.status().await?;

                let v1_seq = v1_status.last_sequence.unwrap_or(0);
                let v2_seq = v2_status.last_sequence.unwrap_or(0);

                if v1_seq >= final_sequence && v2_seq >= final_sequence {
                    info!(
                        validator1_seq = v1_seq,
                        validator2_seq = v2_seq,
                        final_sequence = final_sequence,
                        "Both validators synced past all messages"
                    );
                    validator_synced = true;
                    break;
                }

                info!(
                    validator1_seq = v1_seq,
                    validator2_seq = v2_seq,
                    target = final_sequence,
                    "Waiting for validators..."
                );
            }

            ensure!(
                validator_synced,
                "Validators did not sync past sequence {} within 30s",
                final_sequence
            );

            info!(
                snapshot_seq = snapshot_sequence,
                changeset1_seq = cs1_response.sequence,
                changeset2_seq = final_sequence,
                "Snapshot + changeset flow completed successfully"
            );

            Ok(())
        })
        .await
    }
}
