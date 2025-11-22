//! Event persistence and idempotency tracking.
//!
//! This module provides SQLite-based storage for tracking processed blockchain
//! events to ensure idempotency and enable crash recovery.

use alloy::primitives::B256;
use anyhow::Result;
use rusqlite::{params, Connection};
use tracing::{debug, info};

/// SQLite-based event store for tracking processed events.
///
/// This store provides:
/// - Idempotency checking (prevent duplicate event processing)
/// - Block checkpoint tracking (resume from last processed block)
/// - Crash recovery (persist state across restarts)
#[derive(Debug)]
pub struct EventStore {
    conn: Connection,
}

impl EventStore {
    /// Create a new event store at the specified database path.
    ///
    /// This will create the database and necessary tables if they don't exist.
    ///
    /// # Arguments
    ///
    /// * `db_path` - Path to the `SQLite` database file
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use synddb_chain_monitor::EventStore;
    ///
    /// let store = EventStore::new("./events.db").unwrap();
    /// ```
    pub fn new(db_path: &str) -> Result<Self> {
        let conn = Connection::open(db_path)?;

        // Create tables if they don't exist
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS processed_events (
                tx_hash TEXT PRIMARY KEY,
                block_number INTEGER NOT NULL,
                log_index INTEGER,
                processed_at INTEGER NOT NULL DEFAULT (unixepoch())
            );

            CREATE TABLE IF NOT EXISTS monitor_state (
                key TEXT PRIMARY KEY,
                value INTEGER NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_processed_events_block
            ON processed_events(block_number);
            "#,
        )?;

        info!(db_path = %db_path, "EventStore initialized");
        Ok(Self { conn })
    }

    /// Check if an event has already been processed.
    ///
    /// # Arguments
    ///
    /// * `tx_hash` - The transaction hash to check
    ///
    /// # Returns
    ///
    /// `true` if the event has been processed, `false` otherwise
    pub fn is_processed(&self, tx_hash: &B256) -> Result<bool> {
        let tx_hash_str = format!("{:#x}", tx_hash);
        let exists: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM processed_events WHERE tx_hash = ?1)",
            params![tx_hash_str],
            |row| row.get(0),
        )?;

        if exists {
            debug!(tx_hash = %tx_hash_str, "Event already processed");
        }

        Ok(exists)
    }

    /// Mark an event as processed.
    ///
    /// # Arguments
    ///
    /// * `tx_hash` - The transaction hash
    /// * `block_number` - The block number containing this transaction
    /// * `log_index` - Optional log index within the transaction
    pub fn mark_processed(
        &self,
        tx_hash: &B256,
        block_number: u64,
        log_index: Option<u64>,
    ) -> Result<()> {
        let tx_hash_str = format!("{:#x}", tx_hash);
        self.conn.execute(
            "INSERT OR IGNORE INTO processed_events (tx_hash, block_number, log_index)
             VALUES (?1, ?2, ?3)",
            params![
                tx_hash_str,
                block_number as i64,
                log_index.map(|i| i as i64)
            ],
        )?;

        debug!(
            tx_hash = %tx_hash_str,
            block_number = block_number,
            "Marked event as processed"
        );

        Ok(())
    }

    /// Get the last processed block number.
    ///
    /// Returns `None` if no blocks have been processed yet.
    pub fn get_last_processed_block(&self) -> Result<Option<u64>> {
        let result = self.conn.query_row(
            "SELECT value FROM monitor_state WHERE key = 'last_block'",
            [],
            |row| row.get::<_, i64>(0),
        );

        match result {
            Ok(block) => {
                debug!(last_block = block, "Retrieved last processed block");
                Ok(Some(block as u64))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                debug!("No last processed block found");
                Ok(None)
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Set the last processed block number.
    ///
    /// # Arguments
    ///
    /// * `block` - The block number to save
    pub fn set_last_processed_block(&self, block: u64) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO monitor_state (key, value) VALUES ('last_block', ?1)",
            params![block as i64],
        )?;

        debug!(block = block, "Updated last processed block");
        Ok(())
    }

    /// Get the number of processed events.
    ///
    /// Useful for monitoring and debugging.
    pub fn count_processed_events(&self) -> Result<u64> {
        let count: i64 =
            self.conn
                .query_row("SELECT COUNT(*) FROM processed_events", [], |row| {
                    row.get(0)
                })?;
        Ok(count as u64)
    }

    /// Clear all processed events (useful for testing).
    ///
    /// **Warning**: This will delete all event tracking data.
    #[cfg(test)]
    pub fn clear(&self) -> Result<()> {
        self.conn.execute("DELETE FROM processed_events", [])?;
        self.conn.execute("DELETE FROM monitor_state", [])?;
        info!("EventStore cleared");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_store_initialization() {
        let store = EventStore::new(":memory:").unwrap();
        assert_eq!(store.count_processed_events().unwrap(), 0);
    }

    #[test]
    fn test_mark_and_check_processed() {
        let store = EventStore::new(":memory:").unwrap();
        let tx_hash = B256::from([0x42; 32]);

        // Initially not processed
        assert!(!store.is_processed(&tx_hash).unwrap());

        // Mark as processed
        store.mark_processed(&tx_hash, 12345, Some(0)).unwrap();

        // Now should be processed
        assert!(store.is_processed(&tx_hash).unwrap());
        assert_eq!(store.count_processed_events().unwrap(), 1);
    }

    #[test]
    fn test_duplicate_marking_ignored() {
        let store = EventStore::new(":memory:").unwrap();
        let tx_hash = B256::from([0x99; 32]);

        // Mark twice
        store.mark_processed(&tx_hash, 100, None).unwrap();
        store.mark_processed(&tx_hash, 100, None).unwrap();

        // Should only be counted once
        assert_eq!(store.count_processed_events().unwrap(), 1);
    }

    #[test]
    fn test_last_processed_block() {
        let store = EventStore::new(":memory:").unwrap();

        // Initially no last block
        assert_eq!(store.get_last_processed_block().unwrap(), None);

        // Set last block
        store.set_last_processed_block(5000).unwrap();
        assert_eq!(store.get_last_processed_block().unwrap(), Some(5000));

        // Update last block
        store.set_last_processed_block(5001).unwrap();
        assert_eq!(store.get_last_processed_block().unwrap(), Some(5001));
    }

    #[test]
    fn test_multiple_events() {
        let store = EventStore::new(":memory:").unwrap();

        let tx1 = B256::from([0x01; 32]);
        let tx2 = B256::from([0x02; 32]);
        let tx3 = B256::from([0x03; 32]);

        store.mark_processed(&tx1, 100, Some(0)).unwrap();
        store.mark_processed(&tx2, 100, Some(1)).unwrap();
        store.mark_processed(&tx3, 101, Some(0)).unwrap();

        assert!(store.is_processed(&tx1).unwrap());
        assert!(store.is_processed(&tx2).unwrap());
        assert!(store.is_processed(&tx3).unwrap());
        assert_eq!(store.count_processed_events().unwrap(), 3);
    }

    #[test]
    fn test_clear() {
        let store = EventStore::new(":memory:").unwrap();
        let tx_hash = B256::from([0xAB; 32]);

        store.mark_processed(&tx_hash, 200, None).unwrap();
        store.set_last_processed_block(200).unwrap();

        assert_eq!(store.count_processed_events().unwrap(), 1);
        assert_eq!(store.get_last_processed_block().unwrap(), Some(200));

        store.clear().unwrap();

        assert_eq!(store.count_processed_events().unwrap(), 0);
        assert_eq!(store.get_last_processed_block().unwrap(), None);
    }
}
