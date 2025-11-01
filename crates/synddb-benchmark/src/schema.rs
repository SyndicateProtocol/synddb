use anyhow::Result;
use rusqlite::Connection;

/// Initialize the orderbook database schema
pub fn initialize_schema(conn: &Connection) -> Result<()> {
    // Enable WAL mode (required for SQLite Session Extension)
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;

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
