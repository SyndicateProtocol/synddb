//! State storage for validator sync progress
//!
//! Uses `SQLite` to persist the last processed sequence number and sync metadata.

use anyhow::{Context, Result};
use rusqlite::Connection;
use tracing::{debug, info};

/// Persists validator state for crash recovery
pub struct StateStore {
    conn: Connection,
}

impl StateStore {
    /// Create a new state store with the given database path
    pub fn new(db_path: &str) -> Result<Self> {
        let conn = Connection::open(db_path)
            .context(format!("Failed to open state database: {db_path}"))?;

        // Initialize schema
        conn.execute_batch(
            r"
            CREATE TABLE IF NOT EXISTS validator_state (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL,
                updated_at INTEGER NOT NULL
            );

            -- Insert default values if not present
            -- -1 means never synced
            INSERT OR IGNORE INTO validator_state (key, value, updated_at)
            VALUES ('last_sequence', '-1', strftime('%s', 'now'));

            INSERT OR IGNORE INTO validator_state (key, value, updated_at)
            VALUES ('last_sync_time', '0', strftime('%s', 'now'));
            ",
        )
        .context("Failed to initialize state schema")?;

        info!(path = db_path, "State store initialized");

        Ok(Self { conn })
    }

    /// Create an in-memory state store (for testing)
    pub fn in_memory() -> Result<Self> {
        Self::new(":memory:")
    }

    /// Get the last successfully processed sequence number
    ///
    /// Returns `None` if no messages have been synced yet.
    pub fn last_sequence(&self) -> Result<Option<u64>> {
        let value: String = self
            .conn
            .query_row(
                "SELECT value FROM validator_state WHERE key = 'last_sequence'",
                [],
                |row| row.get(0),
            )
            .context("Failed to get last_sequence")?;

        // Empty string or "-1" means never synced
        if value.is_empty() || value == "-1" {
            return Ok(None);
        }

        Ok(Some(value.parse().context("Invalid last_sequence value")?))
    }

    /// Get the next sequence number to sync
    ///
    /// Returns 0 for a fresh validator, or `last_sequence + 1` otherwise.
    pub fn next_sequence(&self) -> Result<u64> {
        Ok(self.last_sequence()?.map_or(0, |seq| seq + 1))
    }

    /// Update the last successfully processed sequence number
    pub fn set_last_sequence(&self, sequence: u64) -> Result<()> {
        self.conn
            .execute(
                "UPDATE validator_state SET value = ?, updated_at = strftime('%s', 'now') WHERE key = 'last_sequence'",
                [sequence.to_string()],
            )
            .context("Failed to update last_sequence")?;

        debug!(sequence, "Updated last_sequence");

        Ok(())
    }

    /// Get the last sync timestamp (Unix seconds)
    pub fn last_sync_time(&self) -> Result<u64> {
        let value: String = self
            .conn
            .query_row(
                "SELECT value FROM validator_state WHERE key = 'last_sync_time'",
                [],
                |row| row.get(0),
            )
            .context("Failed to get last_sync_time")?;

        value.parse().context("Invalid last_sync_time value")
    }

    /// Update the last sync timestamp
    pub fn set_last_sync_time(&self, timestamp: u64) -> Result<()> {
        self.conn
            .execute(
                "UPDATE validator_state SET value = ?, updated_at = strftime('%s', 'now') WHERE key = 'last_sync_time'",
                [timestamp.to_string()],
            )
            .context("Failed to update last_sync_time")?;

        Ok(())
    }

    /// Record a successful sync operation
    pub fn record_sync(&self, sequence: u64) -> Result<()> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        self.set_last_sequence(sequence)?;
        self.set_last_sync_time(now)?;

        Ok(())
    }

    /// Get the state update timestamp for a given key
    pub fn updated_at(&self, key: &str) -> Result<u64> {
        let value: i64 = self
            .conn
            .query_row(
                "SELECT updated_at FROM validator_state WHERE key = ?",
                [key],
                |row| row.get(0),
            )
            .context("Failed to get updated_at")?;

        Ok(value as u64)
    }
}

impl std::fmt::Debug for StateStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StateStore")
            .field("conn", &"<Connection>")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_store_initialization() {
        let store = StateStore::in_memory().unwrap();

        // Should have default values
        // None means never synced
        assert_eq!(store.last_sequence().unwrap(), None);
        assert_eq!(store.next_sequence().unwrap(), 0);
        assert_eq!(store.last_sync_time().unwrap(), 0);
    }

    #[test]
    fn test_set_and_get_sequence() {
        let store = StateStore::in_memory().unwrap();

        store.set_last_sequence(42).unwrap();
        assert_eq!(store.last_sequence().unwrap(), Some(42));
        assert_eq!(store.next_sequence().unwrap(), 43);

        store.set_last_sequence(100).unwrap();
        assert_eq!(store.last_sequence().unwrap(), Some(100));
        assert_eq!(store.next_sequence().unwrap(), 101);
    }

    #[test]
    fn test_set_and_get_sync_time() {
        let store = StateStore::in_memory().unwrap();

        store.set_last_sync_time(1700000000).unwrap();
        assert_eq!(store.last_sync_time().unwrap(), 1700000000);
    }

    #[test]
    fn test_record_sync() {
        let store = StateStore::in_memory().unwrap();

        // Before sync
        assert_eq!(store.last_sequence().unwrap(), None);
        assert_eq!(store.last_sync_time().unwrap(), 0);

        // Record sync
        store.record_sync(42).unwrap();

        // After sync
        assert_eq!(store.last_sequence().unwrap(), Some(42));
        assert!(store.last_sync_time().unwrap() > 0);
    }

    #[test]
    fn test_updated_at() {
        let store = StateStore::in_memory().unwrap();

        // Get initial updated_at
        let initial = store.updated_at("last_sequence").unwrap();
        assert!(initial > 0);

        // Update and check timestamp changed
        std::thread::sleep(std::time::Duration::from_millis(1100)); // Ensure time advances
        store.set_last_sequence(42).unwrap();

        let updated = store.updated_at("last_sequence").unwrap();
        assert!(updated >= initial);
    }

    #[test]
    fn test_state_persistence() {
        use std::fs;
        use std::path::PathBuf;

        // Use a temp file
        let temp_dir = std::env::temp_dir();
        let db_path: PathBuf = temp_dir.join("test_validator_state.db");
        let db_path_str = db_path.to_str().unwrap();

        // Clean up any previous test run
        let _ = fs::remove_file(&db_path);

        // Create store and set values
        {
            let store = StateStore::new(db_path_str).unwrap();
            store.set_last_sequence(42).unwrap();
            store.set_last_sync_time(1700000000).unwrap();
        }

        // Reopen and verify persistence
        {
            let store = StateStore::new(db_path_str).unwrap();
            assert_eq!(store.last_sequence().unwrap(), Some(42));
            assert_eq!(store.last_sync_time().unwrap(), 1700000000);
        }

        // Clean up
        let _ = fs::remove_file(&db_path);
    }
}
