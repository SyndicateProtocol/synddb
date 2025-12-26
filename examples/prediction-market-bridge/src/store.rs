//! Local cache read operations.
//!
//! All reads come from the local SQLite cache, which is synced from chain events.
//! This provides fast queries but with eventual consistency - the cache may lag
//! behind the on-chain state.

use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

use crate::types::{Account, Market, Outcome, Position, Trade, TradeSide};

/// Store for reading from the local cache.
#[allow(missing_debug_implementations)] // Connection doesn't implement Debug
pub struct Store {
    conn: Connection,
}

impl Store {
    /// Open a store with the given database path.
    pub fn open(path: &str) -> Result<Self> {
        let conn = Connection::open(path)?;
        crate::schema::configure_connection(&conn)?;
        Ok(Self { conn })
    }

    /// Open an in-memory store (for testing).
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        crate::schema::initialize_schema(&conn)?;
        Ok(Self { conn })
    }

    /// Get a reference to the connection (for sync operations).
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    // =========================================================================
    // Market queries
    // =========================================================================

    /// Get a market by ID.
    pub fn get_market(&self, id: &str) -> Result<Option<Market>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, question, resolution_time, resolved, winning_outcome,
                    total_yes_shares, total_no_shares, created_at, resolved_at
             FROM markets WHERE id = ?1",
        )?;

        let result = stmt
            .query_row(params![id], |row| {
                Ok(Market {
                    id: row.get(0)?,
                    question: row.get(1)?,
                    resolution_time: row.get(2)?,
                    resolved: row.get::<_, i64>(3)? != 0,
                    winning_outcome: row
                        .get::<_, Option<i64>>(4)?
                        .and_then(|v| Outcome::from_u8(v as u8)),
                    total_yes_shares: row.get(5)?,
                    total_no_shares: row.get(6)?,
                    created_at: row.get(7)?,
                    resolved_at: row.get(8)?,
                })
            })
            .optional()?;

        Ok(result)
    }

    /// List all markets.
    pub fn list_markets(&self) -> Result<Vec<Market>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, question, resolution_time, resolved, winning_outcome,
                    total_yes_shares, total_no_shares, created_at, resolved_at
             FROM markets ORDER BY created_at DESC",
        )?;

        let markets = stmt
            .query_map([], |row| {
                Ok(Market {
                    id: row.get(0)?,
                    question: row.get(1)?,
                    resolution_time: row.get(2)?,
                    resolved: row.get::<_, i64>(3)? != 0,
                    winning_outcome: row
                        .get::<_, Option<i64>>(4)?
                        .and_then(|v| Outcome::from_u8(v as u8)),
                    total_yes_shares: row.get(5)?,
                    total_no_shares: row.get(6)?,
                    created_at: row.get(7)?,
                    resolved_at: row.get(8)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(markets)
    }

    /// List unresolved markets.
    pub fn list_active_markets(&self) -> Result<Vec<Market>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, question, resolution_time, resolved, winning_outcome,
                    total_yes_shares, total_no_shares, created_at, resolved_at
             FROM markets WHERE resolved = 0 ORDER BY resolution_time ASC",
        )?;

        let markets = stmt
            .query_map([], |row| {
                Ok(Market {
                    id: row.get(0)?,
                    question: row.get(1)?,
                    resolution_time: row.get(2)?,
                    resolved: false,
                    winning_outcome: None,
                    total_yes_shares: row.get(5)?,
                    total_no_shares: row.get(6)?,
                    created_at: row.get(7)?,
                    resolved_at: None,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(markets)
    }

    // =========================================================================
    // Account queries
    // =========================================================================

    /// Get an account by address.
    pub fn get_account(&self, address: &str) -> Result<Option<Account>> {
        let mut stmt = self
            .conn
            .prepare("SELECT address, balance, created_at FROM accounts WHERE address = ?1")?;

        let result = stmt
            .query_row(params![address], |row| {
                Ok(Account {
                    address: row.get(0)?,
                    balance: row.get(1)?,
                    created_at: row.get(2)?,
                })
            })
            .optional()?;

        Ok(result)
    }

    /// Get balance for an account.
    pub fn get_balance(&self, address: &str) -> Result<i64> {
        let balance: i64 = self
            .conn
            .query_row(
                "SELECT balance FROM accounts WHERE address = ?1",
                params![address],
                |row| row.get(0),
            )
            .unwrap_or(0);

        Ok(balance)
    }

    // =========================================================================
    // Position queries
    // =========================================================================

    /// Get a user's position in a market.
    pub fn get_position(&self, user: &str, market_id: &str, outcome: Outcome) -> Result<Option<Position>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, user, market_id, outcome, shares, cost_basis
             FROM positions
             WHERE user = ?1 AND market_id = ?2 AND outcome = ?3",
        )?;

        let result = stmt
            .query_row(params![user, market_id, outcome.as_u8()], |row| {
                Ok(Position {
                    id: row.get(0)?,
                    user: row.get(1)?,
                    market_id: row.get(2)?,
                    outcome: Outcome::from_u8(row.get::<_, i64>(3)? as u8).unwrap_or(Outcome::No),
                    shares: row.get(4)?,
                    cost_basis: row.get(5)?,
                })
            })
            .optional()?;

        Ok(result)
    }

    /// Get all positions for a user.
    pub fn get_user_positions(&self, user: &str) -> Result<Vec<Position>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, user, market_id, outcome, shares, cost_basis
             FROM positions
             WHERE user = ?1 AND shares > 0
             ORDER BY market_id, outcome",
        )?;

        let positions = stmt
            .query_map(params![user], |row| {
                Ok(Position {
                    id: row.get(0)?,
                    user: row.get(1)?,
                    market_id: row.get(2)?,
                    outcome: Outcome::from_u8(row.get::<_, i64>(3)? as u8).unwrap_or(Outcome::No),
                    shares: row.get(4)?,
                    cost_basis: row.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(positions)
    }

    /// Get all positions in a market.
    pub fn get_market_positions(&self, market_id: &str) -> Result<Vec<Position>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, user, market_id, outcome, shares, cost_basis
             FROM positions
             WHERE market_id = ?1 AND shares > 0
             ORDER BY user, outcome",
        )?;

        let positions = stmt
            .query_map(params![market_id], |row| {
                Ok(Position {
                    id: row.get(0)?,
                    user: row.get(1)?,
                    market_id: row.get(2)?,
                    outcome: Outcome::from_u8(row.get::<_, i64>(3)? as u8).unwrap_or(Outcome::No),
                    shares: row.get(4)?,
                    cost_basis: row.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(positions)
    }

    // =========================================================================
    // Trade queries
    // =========================================================================

    /// Get recent trades for a user.
    pub fn get_user_trades(&self, user: &str, limit: usize) -> Result<Vec<Trade>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, user, market_id, outcome, side, shares, price, total, executed_at, message_id
             FROM trades
             WHERE user = ?1
             ORDER BY executed_at DESC
             LIMIT ?2",
        )?;

        let trades = stmt
            .query_map(params![user, limit as i64], |row| {
                Ok(Trade {
                    id: row.get(0)?,
                    user: row.get(1)?,
                    market_id: row.get(2)?,
                    outcome: Outcome::from_u8(row.get::<_, i64>(3)? as u8).unwrap_or(Outcome::No),
                    side: if row.get::<_, String>(4)? == "buy" {
                        TradeSide::Buy
                    } else {
                        TradeSide::Sell
                    },
                    shares: row.get(5)?,
                    price: row.get(6)?,
                    total: row.get(7)?,
                    executed_at: row.get(8)?,
                    message_id: row.get(9)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(trades)
    }

    /// Get recent trades for a market.
    pub fn get_market_trades(&self, market_id: &str, limit: usize) -> Result<Vec<Trade>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, user, market_id, outcome, side, shares, price, total, executed_at, message_id
             FROM trades
             WHERE market_id = ?1
             ORDER BY executed_at DESC
             LIMIT ?2",
        )?;

        let trades = stmt
            .query_map(params![market_id, limit as i64], |row| {
                Ok(Trade {
                    id: row.get(0)?,
                    user: row.get(1)?,
                    market_id: row.get(2)?,
                    outcome: Outcome::from_u8(row.get::<_, i64>(3)? as u8).unwrap_or(Outcome::No),
                    side: if row.get::<_, String>(4)? == "buy" {
                        TradeSide::Buy
                    } else {
                        TradeSide::Sell
                    },
                    shares: row.get(5)?,
                    price: row.get(6)?,
                    total: row.get(7)?,
                    executed_at: row.get(8)?,
                    message_id: row.get(9)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(trades)
    }

    // =========================================================================
    // Analytics queries (demonstrating SQLite strengths)
    // =========================================================================

    /// Calculate portfolio value for a user across all unresolved markets.
    ///
    /// This is a complex aggregation that SQLite handles easily in one query.
    /// In pure message-passing, you'd need to call the contract for each market.
    pub fn get_portfolio_value(&self, user: &str) -> Result<PortfolioSummary> {
        let mut stmt = self.conn.prepare(
            "SELECT
                COUNT(DISTINCT p.market_id) as total_positions,
                COALESCE(SUM(p.cost_basis), 0) as total_cost_basis,
                COALESCE(SUM(p.shares * 50), 0) as estimated_value
             FROM positions p
             JOIN markets m ON p.market_id = m.id
             WHERE p.user = ?1 AND m.resolved = 0 AND p.shares > 0",
        )?;

        let summary = stmt.query_row(params![user], |row| {
            let cost_basis: i64 = row.get(1)?;
            let estimated_value: i64 = row.get(2)?;
            Ok(PortfolioSummary {
                total_positions: row.get(0)?,
                total_cost_basis: cost_basis,
                estimated_value,
                unrealized_pnl: estimated_value - cost_basis,
            })
        })?;

        Ok(summary)
    }

    /// Calculate realized P&L from resolved markets.
    ///
    /// Another complex aggregation - computes profit/loss across all resolved markets.
    pub fn get_realized_pnl(&self, user: &str) -> Result<i64> {
        let pnl: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(SUM(
                    CASE
                        WHEN p.outcome = m.winning_outcome THEN p.shares * 100 - p.cost_basis
                        ELSE -p.cost_basis
                    END
                ), 0)
             FROM positions p
             JOIN markets m ON p.market_id = m.id
             WHERE p.user = ?1 AND m.resolved = 1 AND p.shares > 0",
                params![user],
                |row| row.get(0),
            )
            .unwrap_or(0);

        Ok(pnl)
    }

    /// Get market statistics.
    pub fn get_market_stats(&self, market_id: &str) -> Result<Option<MarketStats>> {
        let mut stmt = self.conn.prepare(
            "SELECT
                m.id,
                m.question,
                COALESCE(SUM(t.total), 0) as total_volume,
                COUNT(DISTINCT t.user) as unique_traders,
                m.total_yes_shares,
                m.total_no_shares
             FROM markets m
             LEFT JOIN trades t ON m.id = t.market_id
             WHERE m.id = ?1
             GROUP BY m.id",
        )?;

        let result = stmt
            .query_row(params![market_id], |row| {
                let yes_shares: i64 = row.get(4)?;
                let no_shares: i64 = row.get(5)?;
                let total = yes_shares + no_shares;
                Ok(MarketStats {
                    market_id: row.get(0)?,
                    question: row.get(1)?,
                    total_volume: row.get(2)?,
                    unique_traders: row.get(3)?,
                    yes_shares,
                    no_shares,
                    yes_percentage: if total > 0 {
                        (yes_shares as f64 / total as f64) * 100.0
                    } else {
                        50.0
                    },
                })
            })
            .optional()?;

        Ok(result)
    }

    /// Get leaderboard of top traders by realized P&L.
    pub fn get_leaderboard(&self, limit: usize) -> Result<Vec<LeaderboardEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT
                p.user,
                COUNT(DISTINCT p.market_id) as markets_traded,
                SUM(CASE
                    WHEN p.outcome = m.winning_outcome THEN p.shares * 100 - p.cost_basis
                    ELSE -p.cost_basis
                END) as total_pnl
             FROM positions p
             JOIN markets m ON p.market_id = m.id
             WHERE m.resolved = 1 AND p.shares > 0
             GROUP BY p.user
             ORDER BY total_pnl DESC
             LIMIT ?1",
        )?;

        let entries = stmt
            .query_map(params![limit as i64], |row| {
                Ok(LeaderboardEntry {
                    user: row.get(0)?,
                    markets_traded: row.get(1)?,
                    total_pnl: row.get(2)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(entries)
    }
}

/// Portfolio summary for a user.
#[derive(Debug, Clone)]
pub struct PortfolioSummary {
    pub total_positions: i64,
    pub total_cost_basis: i64,
    pub estimated_value: i64,
    pub unrealized_pnl: i64,
}

/// Statistics for a market.
#[derive(Debug, Clone)]
pub struct MarketStats {
    pub market_id: String,
    pub question: String,
    pub total_volume: i64,
    pub unique_traders: i64,
    pub yes_shares: i64,
    pub no_shares: i64,
    pub yes_percentage: f64,
}

/// Leaderboard entry.
#[derive(Debug, Clone)]
pub struct LeaderboardEntry {
    pub user: String,
    pub markets_traded: i64,
    pub total_pnl: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_test_data(store: &Store) {
        let conn = store.conn();

        // Insert a market
        conn.execute(
            "INSERT INTO markets (id, question, resolution_time, resolved, total_yes_shares, total_no_shares, created_at)
             VALUES ('0x1234', 'Will BTC hit 100k?', 1800000000, 0, 100, 50, 1700000000)",
            [],
        )
        .unwrap();

        // Insert an account
        conn.execute(
            "INSERT INTO accounts (address, balance, created_at)
             VALUES ('0xalice', 100000, 1700000000)",
            [],
        )
        .unwrap();

        // Insert a position
        conn.execute(
            "INSERT INTO positions (user, market_id, outcome, shares, cost_basis)
             VALUES ('0xalice', '0x1234', 1, 50, 2500)",
            [],
        )
        .unwrap();
    }

    #[test]
    fn test_get_market() {
        let store = Store::open_in_memory().unwrap();
        setup_test_data(&store);

        let market = store.get_market("0x1234").unwrap().unwrap();
        assert_eq!(market.question, "Will BTC hit 100k?");
        assert!(!market.resolved);
        assert_eq!(market.total_yes_shares, 100);
    }

    #[test]
    fn test_get_account() {
        let store = Store::open_in_memory().unwrap();
        setup_test_data(&store);

        let account = store.get_account("0xalice").unwrap().unwrap();
        assert_eq!(account.balance, 100000);
    }

    #[test]
    fn test_get_user_positions() {
        let store = Store::open_in_memory().unwrap();
        setup_test_data(&store);

        let positions = store.get_user_positions("0xalice").unwrap();
        assert_eq!(positions.len(), 1);
        assert_eq!(positions[0].shares, 50);
        assert_eq!(positions[0].outcome, Outcome::Yes);
    }

    #[test]
    fn test_portfolio_value() {
        let store = Store::open_in_memory().unwrap();
        setup_test_data(&store);

        let summary = store.get_portfolio_value("0xalice").unwrap();
        assert_eq!(summary.total_positions, 1);
        assert_eq!(summary.total_cost_basis, 2500);
        assert_eq!(summary.estimated_value, 2500); // 50 shares * 50 cents
        assert_eq!(summary.unrealized_pnl, 0);
    }
}
