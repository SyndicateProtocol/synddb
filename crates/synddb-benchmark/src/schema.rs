use anyhow::Result;
use rusqlite::Connection;

/// Initialize the orderbook database schema
pub fn initialize_schema(conn: &Connection) -> Result<()> {
    // Enable WAL mode (required for SQLite Session Extension)
    conn.pragma_update(None, "journal_mode", "WAL")?;

    // Performance optimizations
    conn.pragma_update(None, "synchronous", "NORMAL")?; // Balance safety and speed
    conn.pragma_update(None, "cache_size", -262144)?; // 256MB cache (increased from 64MB)
    conn.pragma_update(None, "temp_store", "MEMORY")?; // Temp tables in memory
    conn.pragma_update(None, "mmap_size", 30_000_000_000_i64)?; // 30GB mmap (increased for better I/O)

    // Lock and concurrency optimizations
    conn.pragma_update(None, "busy_timeout", 5000)?; // Wait up to 5s for locks instead of immediate fail
    conn.pragma_update(None, "wal_autocheckpoint", 10000)?; // Checkpoint every 10000 pages (reduced frequency)
    conn.pragma_update(None, "journal_size_limit", 67_108_864)?; // 64MB WAL limit

    // Multi-threaded operations (sorting, indexing)
    conn.pragma_update(None, "threads", 4)?;

    // Increase prepared statement cache capacity
    conn.set_prepared_statement_cache_capacity(128);

    // Users table
    conn.execute(
        "CREATE TABLE IF NOT EXISTS users (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            username TEXT NOT NULL UNIQUE,
            created_at INTEGER NOT NULL DEFAULT (unixepoch())
        )",
        [],
    )?;

    // Orders table
    conn.execute(
        "CREATE TABLE IF NOT EXISTS orders (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            user_id INTEGER NOT NULL,
            symbol TEXT NOT NULL,
            side TEXT NOT NULL CHECK(side IN ('buy', 'sell')),
            order_type TEXT NOT NULL CHECK(order_type IN ('limit', 'market')),
            price INTEGER,
            quantity INTEGER NOT NULL,
            filled_quantity INTEGER NOT NULL DEFAULT 0,
            status TEXT NOT NULL DEFAULT 'active' CHECK(status IN ('active', 'filled', 'cancelled', 'partial')),
            created_at INTEGER NOT NULL DEFAULT (unixepoch()),
            updated_at INTEGER NOT NULL DEFAULT (unixepoch()),
            FOREIGN KEY (user_id) REFERENCES users(id)
        )",
        [],
    )?;

    // Trades table
    conn.execute(
        "CREATE TABLE IF NOT EXISTS trades (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            buy_order_id INTEGER NOT NULL,
            sell_order_id INTEGER NOT NULL,
            symbol TEXT NOT NULL,
            price INTEGER NOT NULL,
            quantity INTEGER NOT NULL,
            buyer_id INTEGER NOT NULL,
            seller_id INTEGER NOT NULL,
            executed_at INTEGER NOT NULL DEFAULT (unixepoch()),
            FOREIGN KEY (buy_order_id) REFERENCES orders(id),
            FOREIGN KEY (sell_order_id) REFERENCES orders(id),
            FOREIGN KEY (buyer_id) REFERENCES users(id),
            FOREIGN KEY (seller_id) REFERENCES users(id)
        )",
        [],
    )?;

    // Balances table
    conn.execute(
        "CREATE TABLE IF NOT EXISTS balances (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            user_id INTEGER NOT NULL,
            symbol TEXT NOT NULL,
            amount INTEGER NOT NULL DEFAULT 0,
            locked INTEGER NOT NULL DEFAULT 0,
            updated_at INTEGER NOT NULL DEFAULT (unixepoch()),
            FOREIGN KEY (user_id) REFERENCES users(id),
            UNIQUE(user_id, symbol)
        )",
        [],
    )?;

    // Create indexes for common queries
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_orders_user_id ON orders(user_id)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_orders_symbol_status ON orders(symbol, status)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_orders_status_id ON orders(status, id)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_orders_status_side_id ON orders(status, side, id)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_trades_symbol ON trades(symbol)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_balances_user_id ON balances(user_id)",
        [],
    )?;

    // Set schema version
    conn.pragma_update(None, "user_version", 1)?;

    Ok(())
}

/// Clear all data from tables (but keep schema)
pub fn clear_data(conn: &Connection) -> Result<()> {
    conn.execute("DELETE FROM trades", [])?;
    conn.execute("DELETE FROM orders", [])?;
    conn.execute("DELETE FROM balances", [])?;
    conn.execute("DELETE FROM users", [])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initialize_schema() {
        let conn = Connection::open_in_memory().unwrap();

        // Should succeed
        initialize_schema(&conn).unwrap();

        // Check that all tables were created
        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert!(tables.contains(&"users".to_string()));
        assert!(tables.contains(&"orders".to_string()));
        assert!(tables.contains(&"trades".to_string()));
        assert!(tables.contains(&"balances".to_string()));
    }

    #[test]
    fn test_schema_version() {
        let conn = Connection::open_in_memory().unwrap();
        initialize_schema(&conn).unwrap();

        let version: i32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();

        assert_eq!(version, 1);
    }

    #[test]
    fn test_wal_mode() {
        let conn = Connection::open_in_memory().unwrap();
        initialize_schema(&conn).unwrap();

        let journal_mode: String = conn
            .pragma_query_value(None, "journal_mode", |row| row.get(0))
            .unwrap();

        // In-memory databases might not support WAL, but it should not error
        assert!(journal_mode == "wal" || journal_mode == "memory");
    }

    #[test]
    fn test_indexes_created() {
        let conn = Connection::open_in_memory().unwrap();
        initialize_schema(&conn).unwrap();

        let indexes: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='index' AND name LIKE 'idx_%' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert!(indexes.contains(&"idx_orders_user_id".to_string()));
        assert!(indexes.contains(&"idx_orders_symbol_status".to_string()));
        assert!(indexes.contains(&"idx_trades_symbol".to_string()));
        assert!(indexes.contains(&"idx_balances_user_id".to_string()));
    }

    #[test]
    fn test_clear_data() {
        let conn = Connection::open_in_memory().unwrap();
        initialize_schema(&conn).unwrap();

        // Insert some test data
        conn.execute("INSERT INTO users (username) VALUES ('test_user')", [])
            .unwrap();
        conn.execute(
            "INSERT INTO orders (user_id, symbol, side, order_type, quantity) VALUES (1, 'BTC-USD', 'buy', 'limit', 10)",
            [],
        ).unwrap();

        // Verify data exists
        let user_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0))
            .unwrap();
        assert_eq!(user_count, 1);

        // Clear data
        clear_data(&conn).unwrap();

        // Verify data is cleared
        let user_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0))
            .unwrap();
        assert_eq!(user_count, 0);

        let order_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM orders", [], |row| row.get(0))
            .unwrap();
        assert_eq!(order_count, 0);
    }

    #[test]
    fn test_schema_idempotent() {
        let conn = Connection::open_in_memory().unwrap();

        // Initialize twice - should not error
        initialize_schema(&conn).unwrap();
        initialize_schema(&conn).unwrap();

        // Check our 4 main tables exist (excluding sqlite_sequence)
        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert_eq!(tables.len(), 4); // Should have exactly 4 user tables
        assert!(tables.contains(&"users".to_string()));
        assert!(tables.contains(&"orders".to_string()));
        assert!(tables.contains(&"trades".to_string()));
        assert!(tables.contains(&"balances".to_string()));
    }
}
