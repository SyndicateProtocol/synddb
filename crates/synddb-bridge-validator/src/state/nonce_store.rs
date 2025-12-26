use anyhow::Result;
use rusqlite::Connection;
use std::sync::Mutex;

pub struct NonceStore {
    conn: Mutex<Connection>,
}

impl std::fmt::Debug for NonceStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NonceStore").finish_non_exhaustive()
    }
}

impl NonceStore {
    pub fn new(path: &str) -> Result<Self> {
        let conn = if path == ":memory:" {
            Connection::open_in_memory()?
        } else {
            Connection::open(path)?
        };

        conn.execute(
            "CREATE TABLE IF NOT EXISTS nonces (
                domain BLOB PRIMARY KEY,
                last_nonce INTEGER NOT NULL DEFAULT 0
            )",
            [],
        )?;

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn get_expected_nonce(&self, domain: &[u8; 32]) -> Result<u64> {
        let conn = self.conn.lock().unwrap();
        let nonce: Option<i64> = conn
            .query_row(
                "SELECT last_nonce FROM nonces WHERE domain = ?",
                [domain.as_slice()],
                |row| row.get(0),
            )
            .ok();

        Ok(nonce.map_or(1, |n| n as u64 + 1))
    }

    pub fn consume_nonce(&self, domain: &[u8; 32], nonce: u64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO nonces (domain, last_nonce) VALUES (?, ?)
             ON CONFLICT(domain) DO UPDATE SET last_nonce = excluded.last_nonce",
            rusqlite::params![domain.as_slice(), nonce as i64],
        )?;
        Ok(())
    }

    pub fn sync_from_bridge(&self, domain: &[u8; 32], last_nonce: u64) -> Result<()> {
        self.consume_nonce(domain, last_nonce)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nonce_store() {
        let store = NonceStore::new(":memory:").unwrap();
        let domain = [1u8; 32];

        assert_eq!(store.get_expected_nonce(&domain).unwrap(), 1);

        store.consume_nonce(&domain, 1).unwrap();
        assert_eq!(store.get_expected_nonce(&domain).unwrap(), 2);

        store.consume_nonce(&domain, 2).unwrap();
        assert_eq!(store.get_expected_nonce(&domain).unwrap(), 3);
    }

    #[test]
    fn test_sync_from_bridge() {
        let store = NonceStore::new(":memory:").unwrap();
        let domain = [2u8; 32];

        store.sync_from_bridge(&domain, 100).unwrap();
        assert_eq!(store.get_expected_nonce(&domain).unwrap(), 101);
    }
}
