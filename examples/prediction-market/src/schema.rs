use anyhow::Result;
use rusqlite::Connection;

/// Initialize the prediction market database schema
pub fn initialize_schema(conn: &Connection) -> Result<()> {
    // Enable WAL mode (required for SQLite Session Extension)
    conn.pragma_update(None, "journal_mode", "WAL")?;

    // Performance optimizations
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "cache_size", -65536)?; // 64MB cache
    conn.pragma_update(None, "temp_store", "MEMORY")?;

    // Lock and concurrency
    conn.pragma_update(None, "busy_timeout", 5000)?;

    // Prepared statement cache
    conn.set_prepared_statement_cache_capacity(64);

    // Markets table - binary YES/NO prediction markets
    conn.execute(
        "CREATE TABLE IF NOT EXISTS markets (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            question TEXT NOT NULL,
            description TEXT,
            resolution_time INTEGER NOT NULL,
            outcome TEXT CHECK(outcome IN ('yes', 'no', 'unresolved')) DEFAULT 'unresolved',
            resolved_at INTEGER,
            created_at INTEGER NOT NULL DEFAULT (unixepoch())
        )",
        [],
    )?;

    // Accounts table - user balances
    conn.execute(
        "CREATE TABLE IF NOT EXISTS accounts (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL UNIQUE,
            balance INTEGER NOT NULL DEFAULT 1000000,
            created_at INTEGER NOT NULL DEFAULT (unixepoch())
        )",
        [],
    )?;

    // Positions table - shares held per market/outcome
    conn.execute(
        "CREATE TABLE IF NOT EXISTS positions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            account_id INTEGER NOT NULL,
            market_id INTEGER NOT NULL,
            outcome TEXT NOT NULL CHECK(outcome IN ('yes', 'no')),
            shares INTEGER NOT NULL DEFAULT 0,
            cost_basis INTEGER NOT NULL DEFAULT 0,
            FOREIGN KEY (account_id) REFERENCES accounts(id),
            FOREIGN KEY (market_id) REFERENCES markets(id),
            UNIQUE(account_id, market_id, outcome)
        )",
        [],
    )?;

    // Trades table - trade history
    conn.execute(
        "CREATE TABLE IF NOT EXISTS trades (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            account_id INTEGER NOT NULL,
            market_id INTEGER NOT NULL,
            outcome TEXT NOT NULL CHECK(outcome IN ('yes', 'no')),
            side TEXT NOT NULL CHECK(side IN ('buy', 'sell')),
            shares INTEGER NOT NULL,
            price INTEGER NOT NULL,
            total INTEGER NOT NULL,
            executed_at INTEGER NOT NULL DEFAULT (unixepoch()),
            FOREIGN KEY (account_id) REFERENCES accounts(id),
            FOREIGN KEY (market_id) REFERENCES markets(id)
        )",
        [],
    )?;

    // Inbound deposits from L1 (populated by chain monitor)
    // The chain monitor watches for Deposit events and inserts records here.
    // The application then processes these to credit user accounts.
    conn.execute(
        "CREATE TABLE IF NOT EXISTS inbound_deposits (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            tx_hash TEXT UNIQUE NOT NULL,
            from_address TEXT NOT NULL,
            to_address TEXT NOT NULL,
            amount INTEGER NOT NULL,
            block_number INTEGER NOT NULL,
            log_index INTEGER,
            processed INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER NOT NULL DEFAULT (unixepoch())
        )",
        [],
    )?;

    // Outbound withdrawals to L1 (requests created by users, confirmed by chain monitor)
    // Users request withdrawals which creates a 'pending' record.
    // The sequencer submits to L1 (status -> 'submitted').
    // The chain monitor sees the Withdrawal event and confirms (status -> 'confirmed').
    conn.execute(
        "CREATE TABLE IF NOT EXISTS outbound_withdrawals (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            account_id INTEGER NOT NULL,
            amount INTEGER NOT NULL,
            destination_address TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'pending' CHECK(status IN ('pending', 'submitted', 'confirmed')),
            l1_tx_hash TEXT,
            confirmed_at INTEGER,
            created_at INTEGER NOT NULL DEFAULT (unixepoch()),
            FOREIGN KEY (account_id) REFERENCES accounts(id)
        )",
        [],
    )?;

    // Indexes for common queries
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_positions_account ON positions(account_id)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_positions_market ON positions(market_id)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_trades_account ON trades(account_id)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_trades_market ON trades(market_id)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_deposits_processed ON inbound_deposits(processed)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_withdrawals_status ON outbound_withdrawals(status)",
        [],
    )?;

    conn.pragma_update(None, "user_version", 1)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initialize_schema() {
        let conn = Connection::open_in_memory().unwrap();
        initialize_schema(&conn).unwrap();

        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert!(tables.contains(&"markets".to_string()));
        assert!(tables.contains(&"accounts".to_string()));
        assert!(tables.contains(&"positions".to_string()));
        assert!(tables.contains(&"trades".to_string()));
        assert!(tables.contains(&"inbound_deposits".to_string()));
        assert!(tables.contains(&"outbound_withdrawals".to_string()));
    }

    #[test]
    fn test_schema_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        initialize_schema(&conn).unwrap();
        initialize_schema(&conn).unwrap();
    }
}
