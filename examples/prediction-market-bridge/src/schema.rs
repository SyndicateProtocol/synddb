//! SQLite schema for local cache.
//!
//! This mirrors the on-chain contract state. The local cache is populated
//! by syncing events from the chain, providing fast reads while writes
//! go through the Bridge.

use anyhow::Result;
use rusqlite::Connection;

/// SQL for creating all prediction market tables and indexes.
pub const SCHEMA_SQL: &str = r"
-- Markets table - binary YES/NO prediction markets
-- Synced from MarketCreated and MarketResolved events
CREATE TABLE IF NOT EXISTS markets (
    id TEXT PRIMARY KEY,                  -- Market ID (hex, derived from bytes32)
    question TEXT NOT NULL,
    resolution_time INTEGER NOT NULL,     -- Unix timestamp
    resolved INTEGER NOT NULL DEFAULT 0,  -- Boolean
    winning_outcome INTEGER,              -- 0=NO, 1=YES, NULL=unresolved
    total_yes_shares INTEGER NOT NULL DEFAULT 0,
    total_no_shares INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    resolved_at INTEGER,
    -- Sync metadata
    created_block INTEGER,                -- Block number of MarketCreated event
    resolved_block INTEGER                -- Block number of MarketResolved event
);

-- Accounts table - user balances
-- Synced from Deposit events
CREATE TABLE IF NOT EXISTS accounts (
    address TEXT PRIMARY KEY,             -- Ethereum address (checksummed)
    balance INTEGER NOT NULL DEFAULT 0,   -- Balance in cents
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
);

-- Positions table - share holdings per market/outcome
-- Synced from SharesPurchased and SharesSold events
CREATE TABLE IF NOT EXISTS positions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user TEXT NOT NULL,                   -- User address
    market_id TEXT NOT NULL,
    outcome INTEGER NOT NULL,             -- 0=NO, 1=YES
    shares INTEGER NOT NULL DEFAULT 0,
    cost_basis INTEGER NOT NULL DEFAULT 0,
    UNIQUE(user, market_id, outcome)
);

-- Trades table - trade history
-- Synced from SharesPurchased and SharesSold events
CREATE TABLE IF NOT EXISTS trades (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user TEXT NOT NULL,
    market_id TEXT NOT NULL,
    outcome INTEGER NOT NULL,             -- 0=NO, 1=YES
    side TEXT NOT NULL CHECK(side IN ('buy', 'sell')),
    shares INTEGER NOT NULL,
    price INTEGER NOT NULL,               -- 50 cents (fixed)
    total INTEGER NOT NULL,
    executed_at INTEGER NOT NULL,
    -- Sync metadata
    block_number INTEGER,
    tx_hash TEXT,
    log_index INTEGER,
    message_id TEXT                       -- Bridge message ID if we submitted it
);

-- Bridge messages - track pending/completed messages we submitted
CREATE TABLE IF NOT EXISTS bridge_messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    message_id TEXT UNIQUE,
    message_type TEXT NOT NULL,           -- createMarket, buyShares, etc.
    payload TEXT NOT NULL,                -- JSON payload
    status TEXT NOT NULL DEFAULT 'pending',
    stage INTEGER,                        -- Bridge stage (0-8)
    tx_hash TEXT,
    error_code TEXT,
    error_message TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

-- Sync state - track last synced block for event sync
CREATE TABLE IF NOT EXISTS sync_state (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- Indexes for query performance
CREATE INDEX IF NOT EXISTS idx_positions_user ON positions(user);
CREATE INDEX IF NOT EXISTS idx_positions_market ON positions(market_id);
CREATE INDEX IF NOT EXISTS idx_trades_user ON trades(user);
CREATE INDEX IF NOT EXISTS idx_trades_market ON trades(market_id);
CREATE INDEX IF NOT EXISTS idx_trades_executed ON trades(executed_at DESC);
CREATE INDEX IF NOT EXISTS idx_markets_resolution ON markets(resolution_time);
CREATE INDEX IF NOT EXISTS idx_bridge_messages_status ON bridge_messages(status);
";

/// Configure connection pragmas for performance.
pub fn configure_connection(conn: &Connection) -> Result<()> {
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "cache_size", -65536)?; // 64MB cache
    conn.pragma_update(None, "temp_store", "MEMORY")?;
    conn.pragma_update(None, "busy_timeout", 5000)?;
    conn.set_prepared_statement_cache_capacity(64);
    Ok(())
}

/// Initialize the database schema.
pub fn initialize_schema(conn: &Connection) -> Result<()> {
    configure_connection(conn)?;
    conn.execute_batch(SCHEMA_SQL)?;
    conn.pragma_update(None, "user_version", 1)?;
    Ok(())
}

/// Check if schema is already initialized.
pub fn is_initialized(conn: &Connection) -> Result<bool> {
    let user_version: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    Ok(user_version >= 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initialize_schema() {
        let conn = Connection::open_in_memory().unwrap();
        initialize_schema(&conn).unwrap();

        let tables: Vec<String> = conn
            .prepare(
                "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
            )
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert!(tables.contains(&"markets".to_string()));
        assert!(tables.contains(&"accounts".to_string()));
        assert!(tables.contains(&"positions".to_string()));
        assert!(tables.contains(&"trades".to_string()));
        assert!(tables.contains(&"bridge_messages".to_string()));
        assert!(tables.contains(&"sync_state".to_string()));
    }

    #[test]
    fn test_schema_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        initialize_schema(&conn).unwrap();
        initialize_schema(&conn).unwrap();
    }

    #[test]
    fn test_is_initialized() {
        let conn = Connection::open_in_memory().unwrap();
        assert!(!is_initialized(&conn).unwrap());
        initialize_schema(&conn).unwrap();
        assert!(is_initialized(&conn).unwrap());
    }
}
