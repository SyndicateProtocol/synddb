//! SQLite helper functions

use anyhow::Result;
use rusqlite::Connection;
use std::path::Path;

pub struct SqliteHelper;

impl SqliteHelper {
    /// Open SQLite connection with recommended settings
    pub fn open_connection(path: &Path) -> Result<Connection> {
        let conn = Connection::open(path)?;

        // Enable WAL mode (required for Session Extension)
        conn.execute("PRAGMA journal_mode = WAL", [])?;

        // Set other recommended PRAGMAs
        conn.execute("PRAGMA synchronous = NORMAL", [])?;
        conn.execute("PRAGMA foreign_keys = ON", [])?;

        Ok(conn)
    }

    /// Get current user_version for schema tracking
    pub fn get_user_version(conn: &Connection) -> Result<i32> {
        let version: i32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        Ok(version)
    }

    /// Get database page count
    pub fn get_page_count(conn: &Connection) -> Result<i32> {
        let count: i32 = conn.query_row("PRAGMA page_count", [], |row| row.get(0))?;
        Ok(count)
    }

    /// Get database encoding
    pub fn get_encoding(conn: &Connection) -> Result<String> {
        let encoding: String = conn.query_row("PRAGMA encoding", [], |row| row.get(0))?;
        Ok(encoding)
    }
}
