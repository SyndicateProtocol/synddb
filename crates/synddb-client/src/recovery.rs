//! Failed batch recovery for reliable changeset/snapshot delivery
//!
//! When sending to the sequencer fails after all retries, we save the data
//! to a local SQLite recovery database for retry on next startup or manual recovery.

use crate::session::{Changeset, Snapshot};
use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use std::path::PathBuf;
use tracing::{debug, error, info, warn};

/// Manages recovery storage for failed changesets and snapshots
pub struct FailedBatchRecovery {
    conn: Connection,
}

impl FailedBatchRecovery {
    /// Create or open the recovery database
    ///
    /// Creates a separate SQLite database for storing failed batches.
    /// This database is kept separate from the main application database.
    pub fn new(db_path: PathBuf) -> Result<Self> {
        let conn = Connection::open(db_path).context("Failed to open recovery database")?;

        // Create tables for failed batches
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS failed_changesets (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                sequence INTEGER NOT NULL,
                data BLOB NOT NULL,
                timestamp INTEGER NOT NULL,
                failed_at INTEGER NOT NULL DEFAULT (unixepoch()),
                retry_count INTEGER NOT NULL DEFAULT 0,
                last_error TEXT
            );

            CREATE TABLE IF NOT EXISTS failed_snapshots (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                sequence INTEGER NOT NULL,
                data BLOB NOT NULL,
                timestamp INTEGER NOT NULL,
                failed_at INTEGER NOT NULL DEFAULT (unixepoch()),
                retry_count INTEGER NOT NULL DEFAULT 0,
                last_error TEXT
            );

            CREATE INDEX IF NOT EXISTS idx_failed_changesets_sequence
            ON failed_changesets(sequence);

            CREATE INDEX IF NOT EXISTS idx_failed_snapshots_sequence
            ON failed_snapshots(sequence);
            "#,
        )
        .context("Failed to create recovery tables")?;

        debug!("Initialized failed batch recovery database");

        Ok(Self { conn })
    }

    /// Save a failed changeset for later retry
    pub fn save_failed_changeset(&self, changeset: &Changeset, error: &str) -> Result<()> {
        let timestamp_secs = changeset
            .timestamp
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        self.conn
            .execute(
                "INSERT INTO failed_changesets (sequence, data, timestamp, last_error)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    changeset.sequence as i64,
                    &changeset.data,
                    timestamp_secs,
                    error
                ],
            )
            .context("Failed to save failed changeset")?;

        info!(
            "Persisted failed changeset: sequence={}, size={} bytes",
            changeset.sequence,
            changeset.data.len()
        );

        Ok(())
    }

    /// Save a failed snapshot for later retry
    pub fn save_failed_snapshot(&self, snapshot: &Snapshot, error: &str) -> Result<()> {
        let timestamp_secs = snapshot
            .timestamp
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        self.conn
            .execute(
                "INSERT INTO failed_snapshots (sequence, data, timestamp, last_error)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    snapshot.sequence as i64,
                    &snapshot.data,
                    timestamp_secs,
                    error
                ],
            )
            .context("Failed to save failed snapshot")?;

        info!(
            "Persisted failed snapshot: sequence={}, size={} bytes",
            snapshot.sequence,
            snapshot.data.len()
        );

        Ok(())
    }

    /// Get all failed changesets for retry
    pub fn get_failed_changesets(&self) -> Result<Vec<(i64, Changeset)>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, sequence, data, timestamp FROM failed_changesets ORDER BY sequence",
        )?;

        let changesets = stmt
            .query_map([], |row| {
                let id: i64 = row.get(0)?;
                let sequence: i64 = row.get(1)?;
                let data: Vec<u8> = row.get(2)?;
                let timestamp_secs: i64 = row.get(3)?;

                let timestamp =
                    std::time::UNIX_EPOCH + std::time::Duration::from_secs(timestamp_secs as u64);

                Ok((
                    id,
                    Changeset {
                        data,
                        sequence: sequence as u64,
                        timestamp,
                    },
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        debug!("Retrieved {} failed changesets", changesets.len());

        Ok(changesets)
    }

    /// Get all failed snapshots for retry
    pub fn get_failed_snapshots(&self) -> Result<Vec<(i64, Snapshot)>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, sequence, data, timestamp FROM failed_snapshots ORDER BY sequence",
        )?;

        let snapshots = stmt
            .query_map([], |row| {
                let id: i64 = row.get(0)?;
                let sequence: i64 = row.get(1)?;
                let data: Vec<u8> = row.get(2)?;
                let timestamp_secs: i64 = row.get(3)?;

                let timestamp =
                    std::time::UNIX_EPOCH + std::time::Duration::from_secs(timestamp_secs as u64);

                Ok((
                    id,
                    Snapshot {
                        data,
                        sequence: sequence as u64,
                        timestamp,
                    },
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        debug!("Retrieved {} failed snapshots", snapshots.len());

        Ok(snapshots)
    }

    /// Remove a changeset after successful retry
    pub fn remove_changeset(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM failed_changesets WHERE id = ?1", params![id])?;
        debug!("Removed persisted changeset id={}", id);
        Ok(())
    }

    /// Remove a snapshot after successful retry
    pub fn remove_snapshot(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM failed_snapshots WHERE id = ?1", params![id])?;
        debug!("Removed persisted snapshot id={}", id);
        Ok(())
    }

    /// Increment retry count for a changeset
    pub fn increment_changeset_retry(&self, id: i64, error: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE failed_changesets SET retry_count = retry_count + 1, last_error = ?2 WHERE id = ?1",
            params![id, error],
        )?;
        Ok(())
    }

    /// Increment retry count for a snapshot
    pub fn increment_snapshot_retry(&self, id: i64, error: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE failed_snapshots SET retry_count = retry_count + 1, last_error = ?2 WHERE id = ?1",
            params![id, error],
        )?;
        Ok(())
    }

    /// Get count of failed batches
    pub fn get_failed_counts(&self) -> Result<(usize, usize)> {
        let changesets: usize =
            self.conn
                .query_row("SELECT COUNT(*) FROM failed_changesets", [], |row| {
                    row.get(0)
                })?;

        let snapshots: usize =
            self.conn
                .query_row("SELECT COUNT(*) FROM failed_snapshots", [], |row| {
                    row.get(0)
                })?;

        Ok((changesets, snapshots))
    }

    /// Clean up old failed batches (older than specified days)
    pub fn cleanup_old_failures(&self, days: u32) -> Result<(usize, usize)> {
        let cutoff_time = std::time::SystemTime::now()
            - std::time::Duration::from_secs(days as u64 * 24 * 60 * 60);
        let cutoff_secs = cutoff_time
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let changesets_deleted = self.conn.execute(
            "DELETE FROM failed_changesets WHERE failed_at < ?1",
            params![cutoff_secs],
        )?;

        let snapshots_deleted = self.conn.execute(
            "DELETE FROM failed_snapshots WHERE failed_at < ?1",
            params![cutoff_secs],
        )?;

        if changesets_deleted > 0 || snapshots_deleted > 0 {
            info!(
                "Cleaned up old failures: {} changesets, {} snapshots",
                changesets_deleted, snapshots_deleted
            );
        }

        Ok((changesets_deleted, snapshots_deleted))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    #[test]
    fn test_recovery_roundtrip() {
        let temp_dir = std::env::temp_dir();
        let db_path = temp_dir.join(format!("test_recovery_{}.db", uuid::Uuid::new_v4()));

        let recovery = FailedBatchRecovery::new(db_path.clone()).unwrap();

        // Create test changeset
        let changeset = Changeset {
            data: vec![1, 2, 3, 4],
            sequence: 42,
            timestamp: SystemTime::now(),
        };

        // Save it
        recovery
            .save_failed_changeset(&changeset, "Test error")
            .unwrap();

        // Retrieve it
        let failed = recovery.get_failed_changesets().unwrap();
        assert_eq!(failed.len(), 1);

        let (id, retrieved) = &failed[0];
        assert_eq!(retrieved.sequence, 42);
        assert_eq!(retrieved.data, vec![1, 2, 3, 4]);

        // Remove it
        recovery.remove_changeset(*id).unwrap();

        let failed_after = recovery.get_failed_changesets().unwrap();
        assert_eq!(failed_after.len(), 0);

        // Cleanup
        std::fs::remove_file(db_path).ok();
    }

    #[test]
    fn test_retry_counting() {
        let temp_dir = std::env::temp_dir();
        let db_path = temp_dir.join(format!("test_retry_{}.db", uuid::Uuid::new_v4()));

        let recovery = FailedBatchRecovery::new(db_path.clone()).unwrap();

        let changeset = Changeset {
            data: vec![1, 2, 3],
            sequence: 1,
            timestamp: SystemTime::now(),
        };

        recovery
            .save_failed_changeset(&changeset, "Initial error")
            .unwrap();

        let failed = recovery.get_failed_changesets().unwrap();
        let (id, _) = failed[0];

        // Increment retry count
        recovery
            .increment_changeset_retry(id, "Retry error 1")
            .unwrap();
        recovery
            .increment_changeset_retry(id, "Retry error 2")
            .unwrap();

        // Check counts
        let (changeset_count, _) = recovery.get_failed_counts().unwrap();
        assert_eq!(changeset_count, 1);

        // Cleanup
        std::fs::remove_file(db_path).ok();
    }
}
