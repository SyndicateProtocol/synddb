//! Audit trail verification for changeset consistency.
//!
//! This module provides functionality to verify that changesets are consistent
//! with snapshots, enabling a "changeset-centric" architecture where all state
//! transitions are auditable even when snapshots are used for efficiency.
//!
//! # Background
//!
//! `SQLite` changesets are invertible - an UPDATE that changes A→B can be inverted
//! to produce a changeset that changes B→A. This property enables verification:
//!
//! ```text
//! Given:
//!   - Snapshot at sequence N (state S_N)
//!   - Changeset at sequence N-1 that we couldn't apply (schema mismatch)
//!
//! We can verify by:
//!   1. Apply inverted changeset to S_N → should produce S_{N-1}
//!   2. Verify the pre-image in original changeset matches S_{N-1}
//! ```
//!
//! # Components
//!
//! - [`PendingChangesetStore`] - Persists changesets that couldn't be applied
//! - [`invert_changeset`] - Inverts a changeset for reverse application
//! - [`verify_changeset_chain`] - Verifies changesets against a snapshot
//!
//! # Integration Guide
//!
//! To integrate audit trail verification into the validator:
//!
//! ```text
//! 1. Create a PendingChangesetStore with its own SQLite connection:
//!    let store_conn = Connection::open("pending_changesets.db")?;
//!    let pending_store = PendingChangesetStore::new(store_conn)?;
//!
//! 2. When apply_message fails with schema mismatch, store the changeset:
//!    match applier.apply_message(&message) {
//!        Err(e) if e.to_string().contains("Schema mismatch") => {
//!            pending_store.store(&PendingChangeset {
//!                sequence: message.sequence,
//!                data: message.payload.clone(),
//!                reason: DeferralReason::MissingTable("...".into()),
//!            })?;
//!        }
//!        other => other?,
//!    }
//!
//! 3. After applying a snapshot, verify pending changesets:
//!    if message.message_type == MessageType::Snapshot {
//!        let pending = pending_store.get_all()?;
//!        let result = verify_changeset_chain(&applier.conn, &pending)?;
//!        pending_store.clear_up_to(snapshot_sequence)?;
//!    }
//! ```
//!
//! # Limitations
//!
//! The current `verify_changeset_chain` implementation verifies each changeset
//! independently against the current state. This means:
//!
//! - Only the most recent changeset (whose post-state matches the snapshot) will verify
//! - Earlier changesets in the chain may fail verification
//!
//! For complete chain verification, a more sophisticated approach would be needed
//! that maintains state across verifications.

use anyhow::{Context, Result};
use rusqlite::Connection;
use tracing::{debug, info, warn};

/// A changeset that was received but couldn't be applied due to schema mismatch.
///
/// These are stored for later verification once a snapshot provides the schema.
#[derive(Debug, Clone)]
pub struct PendingChangeset {
    /// The sequence number of this changeset
    pub sequence: u64,
    /// The raw changeset data (compressed)
    pub data: Vec<u8>,
    /// Why this changeset was deferred
    pub reason: DeferralReason,
}

/// Why a changeset was deferred for later verification
#[derive(Debug, Clone)]
pub enum DeferralReason {
    /// Table referenced in changeset doesn't exist
    MissingTable(String),
    /// Column count mismatch between changeset and target
    ColumnMismatch {
        table: String,
        expected: i32,
        actual: i32,
    },
}

/// Inverts a changeset, producing a new changeset that reverses the operations.
///
/// For example, if the original changeset contains:
/// - INSERT row → inverted has DELETE row
/// - DELETE row → inverted has INSERT row
/// - UPDATE A→B → inverted has UPDATE B→A
///
/// # Arguments
/// * `changeset` - The changeset data to invert
///
/// # Returns
/// The inverted changeset data
pub fn invert_changeset(changeset: &[u8]) -> Result<Vec<u8>> {
    use rusqlite::session::invert_strm;
    use std::io::Cursor;

    let mut input = Cursor::new(changeset);
    let mut output = Vec::new();

    invert_strm(&mut input, &mut output).map_err(|e| anyhow::anyhow!("Failed to invert: {e}"))?;

    Ok(output)
}

/// Verifies that a sequence of changesets is consistent with a snapshot.
///
/// This is the core audit function. Given:
/// - A snapshot at sequence N
/// - Changesets from sequence M to N-1 (that couldn't be applied forward)
///
/// It verifies that applying the inverted changesets in reverse order
/// produces states consistent with each changeset's pre-images.
///
/// # Future Implementation
///
/// ```text
/// fn verify_changeset_chain(
///     snapshot_conn: &Connection,      // Database restored from snapshot
///     snapshot_seq: u64,                // Sequence of snapshot
///     pending: &[PendingChangeset],     // Changesets to verify
/// ) -> Result<AuditResult> {
///     // 1. Sort pending by sequence descending
///     // 2. For each changeset (N-1, N-2, ... M):
///     //    a. Invert the changeset
///     //    b. Apply inverted changeset to current state
///     //    c. Verify pre-images match
///     // 3. Return audit result with any discrepancies
/// }
/// ```
/// Result of audit verification
#[derive(Debug)]
pub struct AuditResult {
    /// Changesets that were successfully verified
    pub verified: Vec<u64>,
    /// Changesets with verification failures
    pub failed: Vec<AuditFailure>,
}

/// A changeset that failed audit verification
#[derive(Debug)]
pub struct AuditFailure {
    /// The sequence number
    pub sequence: u64,
    /// Description of the failure
    pub reason: String,
}

/// Persistent storage for pending changesets awaiting verification.
///
/// When a changeset cannot be applied due to schema mismatch, it is stored here
/// until a snapshot arrives that provides the correct schema. The changesets can
/// then be verified using inversion.
pub struct PendingChangesetStore {
    conn: Connection,
}

impl PendingChangesetStore {
    /// Create a new store, initializing the schema if needed.
    pub fn new(conn: Connection) -> Result<Self> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS pending_changesets (
                sequence INTEGER PRIMARY KEY,
                data BLOB NOT NULL,
                reason TEXT NOT NULL,
                created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
            )",
        )
        .context("Failed to create pending_changesets table")?;

        Ok(Self { conn })
    }

    /// Store a pending changeset for later verification.
    pub fn store(&self, changeset: &PendingChangeset) -> Result<()> {
        let reason_json = match &changeset.reason {
            DeferralReason::MissingTable(table) => {
                format!(r#"{{"type":"missing_table","table":"{}"}}"#, table)
            }
            DeferralReason::ColumnMismatch {
                table,
                expected,
                actual,
            } => {
                format!(
                    r#"{{"type":"column_mismatch","table":"{}","expected":{},"actual":{}}}"#,
                    table, expected, actual
                )
            }
        };

        self.conn
            .execute(
                "INSERT OR REPLACE INTO pending_changesets (sequence, data, reason) VALUES (?, ?, ?)",
                rusqlite::params![changeset.sequence, &changeset.data, reason_json],
            )
            .context("Failed to store pending changeset")?;

        debug!(sequence = changeset.sequence, "Stored pending changeset");
        Ok(())
    }

    /// Get all pending changesets in sequence order.
    pub fn get_all(&self) -> Result<Vec<PendingChangeset>> {
        let mut stmt = self
            .conn
            .prepare("SELECT sequence, data, reason FROM pending_changesets ORDER BY sequence")?;

        let changesets = stmt
            .query_map([], |row| {
                let sequence: u64 = row.get(0)?;
                let data: Vec<u8> = row.get(1)?;
                let reason_json: String = row.get(2)?;

                // Parse reason (simplified - in production use serde_json)
                let reason = if reason_json.contains("missing_table") {
                    let table = reason_json
                        .split("\"table\":\"")
                        .nth(1)
                        .and_then(|s| s.split('"').next())
                        .unwrap_or("unknown")
                        .to_string();
                    DeferralReason::MissingTable(table)
                } else {
                    DeferralReason::MissingTable("unknown".to_string())
                };

                Ok(PendingChangeset {
                    sequence,
                    data,
                    reason,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(changesets)
    }

    /// Get pending changesets in a sequence range (inclusive).
    pub fn get_range(&self, start: u64, end: u64) -> Result<Vec<PendingChangeset>> {
        let mut stmt = self.conn.prepare(
            "SELECT sequence, data, reason FROM pending_changesets
             WHERE sequence >= ? AND sequence <= ?
             ORDER BY sequence",
        )?;

        let changesets = stmt
            .query_map(rusqlite::params![start, end], |row| {
                let sequence: u64 = row.get(0)?;
                let data: Vec<u8> = row.get(1)?;
                let reason_json: String = row.get(2)?;

                let reason = if reason_json.contains("missing_table") {
                    let table = reason_json
                        .split("\"table\":\"")
                        .nth(1)
                        .and_then(|s| s.split('"').next())
                        .unwrap_or("unknown")
                        .to_string();
                    DeferralReason::MissingTable(table)
                } else {
                    DeferralReason::MissingTable("unknown".to_string())
                };

                Ok(PendingChangeset {
                    sequence,
                    data,
                    reason,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(changesets)
    }

    /// Remove pending changesets up to and including the given sequence.
    ///
    /// Called after a snapshot has been applied and verified.
    pub fn clear_up_to(&self, sequence: u64) -> Result<usize> {
        let count = self
            .conn
            .execute(
                "DELETE FROM pending_changesets WHERE sequence <= ?",
                [sequence],
            )
            .context("Failed to clear pending changesets")?;

        if count > 0 {
            info!(
                cleared = count,
                up_to_sequence = sequence,
                "Cleared verified pending changesets"
            );
        }

        Ok(count)
    }

    /// Get the count of pending changesets.
    pub fn count(&self) -> Result<u64> {
        self.conn
            .query_row("SELECT COUNT(*) FROM pending_changesets", [], |row| {
                row.get(0)
            })
            .context("Failed to count pending changesets")
    }

    /// Get the sequence range of pending changesets.
    pub fn sequence_range(&self) -> Result<Option<(u64, u64)>> {
        self.conn
            .query_row(
                "SELECT MIN(sequence), MAX(sequence) FROM pending_changesets",
                [],
                |row| {
                    let min: Option<u64> = row.get(0)?;
                    let max: Option<u64> = row.get(1)?;
                    Ok(min.zip(max))
                },
            )
            .context("Failed to get sequence range")
    }
}

impl std::fmt::Debug for PendingChangesetStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingChangesetStore")
            .field("conn", &"<Connection>")
            .finish()
    }
}

/// Verify a chain of pending changesets against the current database state.
///
/// This function verifies that the pending changesets are consistent with the
/// database state after a snapshot has been applied. It does this by:
///
/// 1. For each changeset (in reverse sequence order):
///    a. Invert the changeset
///    b. Attempt to apply the inverted changeset
///    c. If successful, the changeset is verified
///    d. Re-apply the original to restore state
///
/// Note: This modifies the database temporarily during verification.
/// For production use, consider using a separate verification database.
pub fn verify_changeset_chain(
    conn: &Connection,
    pending: &[PendingChangeset],
) -> Result<AuditResult> {
    use rusqlite::session::ConflictAction;
    use std::io::Cursor;

    let mut verified = Vec::new();
    let mut failed = Vec::new();

    // Process in reverse order (most recent first)
    let mut sorted: Vec<_> = pending.iter().collect();
    sorted.sort_by(|a, b| b.sequence.cmp(&a.sequence));

    for changeset in sorted {
        let seq = changeset.sequence;
        debug!(sequence = seq, "Verifying changeset");

        // Invert the changeset
        let inverted = match invert_changeset(&changeset.data) {
            Ok(inv) => inv,
            Err(e) => {
                failed.push(AuditFailure {
                    sequence: seq,
                    reason: format!("Failed to invert: {e}"),
                });
                continue;
            }
        };

        // Try to apply the inverted changeset
        let mut cursor = Cursor::new(&inverted);
        let apply_result = conn.apply_strm(
            &mut cursor,
            None::<fn(&str) -> bool>,
            move |conflict, _item| {
                warn!(sequence = seq, ?conflict, "Conflict during verification");
                ConflictAction::SQLITE_CHANGESET_ABORT
            },
        );

        match apply_result {
            Ok(()) => {
                // Successfully applied inverted changeset - now restore by applying original
                let mut cursor = Cursor::new(&changeset.data);
                if let Err(e) = conn.apply_strm(&mut cursor, None::<fn(&str) -> bool>, |_, _| {
                    ConflictAction::SQLITE_CHANGESET_ABORT
                }) {
                    warn!(
                        sequence = seq,
                        error = %e,
                        "Failed to restore after verification"
                    );
                }
                verified.push(seq);
            }
            Err(e) => {
                failed.push(AuditFailure {
                    sequence: seq,
                    reason: format!("Inverted changeset failed to apply: {e}"),
                });
            }
        }
    }

    info!(
        verified_count = verified.len(),
        failed_count = failed.len(),
        "Changeset chain verification complete"
    );

    Ok(AuditResult { verified, failed })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{session::Session, Connection};

    #[test]
    fn test_invert_changeset() {
        // Create a database and make a change
        let conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, val TEXT)", [])
            .unwrap();
        conn.execute("INSERT INTO t VALUES (1, 'hello')", [])
            .unwrap();

        // Capture an UPDATE changeset
        let mut session = Session::new(&conn).unwrap();
        session.attach(None::<&str>).unwrap();
        conn.execute("UPDATE t SET val = 'world' WHERE id = 1", [])
            .unwrap();

        let mut original = Vec::new();
        session.changeset_strm(&mut original).unwrap();
        assert!(!original.is_empty());

        // Invert the changeset
        let inverted = invert_changeset(&original).unwrap();
        assert!(!inverted.is_empty());

        // The inverted changeset should be different (UPDATE world→hello)
        assert_ne!(original, inverted);
    }

    #[test]
    fn test_invert_roundtrip() {
        // Inverting twice should give back the original
        let conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, val TEXT)", [])
            .unwrap();
        conn.execute("INSERT INTO t VALUES (1, 'hello')", [])
            .unwrap();

        let mut session = Session::new(&conn).unwrap();
        session.attach(None::<&str>).unwrap();
        conn.execute("UPDATE t SET val = 'world' WHERE id = 1", [])
            .unwrap();

        let mut original = Vec::new();
        session.changeset_strm(&mut original).unwrap();

        // Invert twice
        let inverted = invert_changeset(&original).unwrap();
        let double_inverted = invert_changeset(&inverted).unwrap();

        // Should match original
        assert_eq!(original, double_inverted);
    }

    #[test]
    fn test_apply_inverted_reverses_change() {
        use rusqlite::session::ConflictAction;

        // Create a database with initial state
        let conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, val TEXT)", [])
            .unwrap();
        conn.execute("INSERT INTO t VALUES (1, 'hello')", [])
            .unwrap();

        // Capture an UPDATE changeset
        let mut session = Session::new(&conn).unwrap();
        session.attach(None::<&str>).unwrap();
        conn.execute("UPDATE t SET val = 'world' WHERE id = 1", [])
            .unwrap();

        let mut changeset = Vec::new();
        session.changeset_strm(&mut changeset).unwrap();

        // Now val = 'world'
        let val: String = conn
            .query_row("SELECT val FROM t WHERE id = 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(val, "world");

        // Invert and apply
        let inverted = invert_changeset(&changeset).unwrap();
        let mut cursor = std::io::Cursor::new(&inverted);
        conn.apply_strm(&mut cursor, None::<fn(&str) -> bool>, |_, _| {
            ConflictAction::SQLITE_CHANGESET_ABORT
        })
        .unwrap();

        // Now val should be back to 'hello'
        let val: String = conn
            .query_row("SELECT val FROM t WHERE id = 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(val, "hello");
    }

    #[test]
    fn test_pending_changeset_store_basic() {
        let conn = Connection::open_in_memory().unwrap();
        let store = PendingChangesetStore::new(conn).unwrap();

        // Initially empty
        assert_eq!(store.count().unwrap(), 0);
        assert!(store.sequence_range().unwrap().is_none());

        // Store a pending changeset
        let pending = PendingChangeset {
            sequence: 10,
            data: vec![1, 2, 3, 4],
            reason: DeferralReason::MissingTable("users".to_string()),
        };
        store.store(&pending).unwrap();

        assert_eq!(store.count().unwrap(), 1);
        assert_eq!(store.sequence_range().unwrap(), Some((10, 10)));

        // Retrieve it
        let all = store.get_all().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].sequence, 10);
        assert_eq!(all[0].data, vec![1, 2, 3, 4]);
    }

    #[test]
    fn test_pending_changeset_store_multiple() {
        let conn = Connection::open_in_memory().unwrap();
        let store = PendingChangesetStore::new(conn).unwrap();

        // Store multiple changesets
        for seq in [5, 10, 15, 20] {
            store
                .store(&PendingChangeset {
                    sequence: seq,
                    data: vec![seq as u8],
                    reason: DeferralReason::MissingTable("t".to_string()),
                })
                .unwrap();
        }

        assert_eq!(store.count().unwrap(), 4);
        assert_eq!(store.sequence_range().unwrap(), Some((5, 20)));

        // Get range
        let range = store.get_range(10, 15).unwrap();
        assert_eq!(range.len(), 2);
        assert_eq!(range[0].sequence, 10);
        assert_eq!(range[1].sequence, 15);
    }

    #[test]
    fn test_pending_changeset_store_clear() {
        let conn = Connection::open_in_memory().unwrap();
        let store = PendingChangesetStore::new(conn).unwrap();

        for seq in [5, 10, 15, 20] {
            store
                .store(&PendingChangeset {
                    sequence: seq,
                    data: vec![seq as u8],
                    reason: DeferralReason::MissingTable("t".to_string()),
                })
                .unwrap();
        }

        // Clear up to sequence 15
        let cleared = store.clear_up_to(15).unwrap();
        assert_eq!(cleared, 3); // sequences 5, 10, 15

        // Only sequence 20 remains
        assert_eq!(store.count().unwrap(), 1);
        let remaining = store.get_all().unwrap();
        assert_eq!(remaining[0].sequence, 20);
    }

    #[test]
    fn test_verify_changeset_chain_success() {
        // Create a database with some data
        let conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, val TEXT)", [])
            .unwrap();
        conn.execute("INSERT INTO t VALUES (1, 'final')", [])
            .unwrap();

        // Create a changeset that represents a change TO this state
        // We need to create a changeset from 'initial' -> 'final'
        // and verify it against the 'final' state

        // First, create a temporary database to generate the changeset
        let temp = Connection::open_in_memory().unwrap();
        temp.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, val TEXT)", [])
            .unwrap();
        temp.execute("INSERT INTO t VALUES (1, 'initial')", [])
            .unwrap();

        let mut session = Session::new(&temp).unwrap();
        session.attach(None::<&str>).unwrap();
        temp.execute("UPDATE t SET val = 'final' WHERE id = 1", [])
            .unwrap();

        let mut changeset_data = Vec::new();
        session.changeset_strm(&mut changeset_data).unwrap();

        let pending = vec![PendingChangeset {
            sequence: 1,
            data: changeset_data,
            reason: DeferralReason::MissingTable("t".to_string()),
        }];

        // Verify the chain - should succeed because:
        // 1. Invert (final->initial) applied to 'final' state works
        // 2. Re-apply (initial->final) restores state
        let result = verify_changeset_chain(&conn, &pending).unwrap();
        assert_eq!(result.verified.len(), 1);
        assert_eq!(result.failed.len(), 0);
        assert!(result.verified.contains(&1));

        // State should be unchanged after verification
        let val: String = conn
            .query_row("SELECT val FROM t WHERE id = 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(val, "final");
    }

    #[test]
    fn test_verify_changeset_chain_limitation() {
        // This test documents the limitation of the current verification approach:
        // Each changeset is verified independently, so only the most recent
        // changeset (the one whose post-state matches the snapshot) will verify.
        //
        // For proper chain verification, a more sophisticated approach would be
        // needed that doesn't restore state between each verification.

        // Create final state (val = 100)
        let conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, val INTEGER)", [])
            .unwrap();
        conn.execute("INSERT INTO t VALUES (1, 100)", []).unwrap();

        // Create changesets representing: 0 -> 50 -> 100
        let temp = Connection::open_in_memory().unwrap();
        temp.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, val INTEGER)", [])
            .unwrap();
        temp.execute("INSERT INTO t VALUES (1, 0)", []).unwrap();

        // Changeset 1: 0 -> 50
        let mut session1 = Session::new(&temp).unwrap();
        session1.attach(None::<&str>).unwrap();
        temp.execute("UPDATE t SET val = 50 WHERE id = 1", [])
            .unwrap();
        let mut cs1 = Vec::new();
        session1.changeset_strm(&mut cs1).unwrap();

        // Changeset 2: 50 -> 100
        let mut session2 = Session::new(&temp).unwrap();
        session2.attach(None::<&str>).unwrap();
        temp.execute("UPDATE t SET val = 100 WHERE id = 1", [])
            .unwrap();
        let mut cs2 = Vec::new();
        session2.changeset_strm(&mut cs2).unwrap();

        let pending = vec![
            PendingChangeset {
                sequence: 1,
                data: cs1,
                reason: DeferralReason::MissingTable("t".to_string()),
            },
            PendingChangeset {
                sequence: 2,
                data: cs2,
                reason: DeferralReason::MissingTable("t".to_string()),
            },
        ];

        // Current behavior: Only the most recent changeset (cs2: 50->100) can verify
        // because its post-state (100) matches the snapshot state.
        // cs1 (0->50) cannot verify because after cs2 verification, state is 100,
        // but cs1's inverted form expects state 50.
        let result = verify_changeset_chain(&conn, &pending).unwrap();

        // cs2 verifies (its post-state matches snapshot)
        assert!(result.verified.contains(&2), "cs2 should verify");

        // cs1 fails (its post-state 50 doesn't match current state 100)
        assert_eq!(result.failed.len(), 1, "cs1 should fail in current impl");

        // State should be unchanged
        let val: i32 = conn
            .query_row("SELECT val FROM t WHERE id = 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(val, 100);
    }
}
