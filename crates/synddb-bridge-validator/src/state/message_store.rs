use anyhow::Result;
use rusqlite::Connection;
use std::sync::Mutex;

pub struct MessageStore {
    conn: Mutex<Connection>,
}

impl MessageStore {
    pub fn new(path: &str) -> Result<Self> {
        let conn = if path == ":memory:" {
            Connection::open_in_memory()?
        } else {
            Connection::open(path)?
        };

        conn.execute(
            "CREATE TABLE IF NOT EXISTS processed_messages (
                message_id BLOB PRIMARY KEY,
                processed_at INTEGER NOT NULL
            )",
            [],
        )?;

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn is_processed(&self, message_id: &[u8; 32]) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let exists: bool = conn
            .query_row(
                "SELECT 1 FROM processed_messages WHERE message_id = ?",
                [message_id.as_slice()],
                |_| Ok(true),
            )
            .unwrap_or(false);
        Ok(exists)
    }

    pub fn mark_processed(&self, message_id: &[u8; 32]) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        conn.execute(
            "INSERT OR IGNORE INTO processed_messages (message_id, processed_at) VALUES (?, ?)",
            rusqlite::params![message_id.as_slice(), now as i64],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_store() {
        let store = MessageStore::new(":memory:").unwrap();
        let message_id = [1u8; 32];

        assert!(!store.is_processed(&message_id).unwrap());

        store.mark_processed(&message_id).unwrap();
        assert!(store.is_processed(&message_id).unwrap());

        // Marking again should not error
        store.mark_processed(&message_id).unwrap();
        assert!(store.is_processed(&message_id).unwrap());
    }
}
