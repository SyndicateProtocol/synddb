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
//! # Future Implementation
//!
//! This module is a placeholder for the audit verification system.
//! Key components to implement:
//!
//! 1. `AuditLog` - Stores changesets that couldn't be applied due to schema mismatch
//! 2. `verify_changeset_chain` - Verifies a sequence of changesets against a snapshot
//! 3. `reconstruct_state` - Uses inversion to reconstruct historical states

use anyhow::Result;

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
}
