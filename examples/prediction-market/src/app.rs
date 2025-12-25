use anyhow::Result;
use rusqlite::Connection;
use synddb_client::SyndDB;

use crate::{
    bridge::{self, Withdrawal},
    market::{self, Market},
    schema,
    trading::{self, Position, Trade},
};

/// Prediction market application with optional `SyndDB` replication
///
/// # With Replication (recommended for production)
///
/// ```rust,no_run
/// use prediction_market::app::PredictionMarket;
///
/// let app = PredictionMarket::new("market.db", Some("http://sequencer:8433"))?;
///
/// // All operations are automatically replicated
/// app.create_account("alice")?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// # Without Replication (for testing/development)
///
/// ```rust,no_run
/// use prediction_market::app::PredictionMarket;
///
/// let app = PredictionMarket::new("market.db", None)?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[allow(missing_debug_implementations)]
pub struct PredictionMarket {
    /// When replicated, `SyndDB` manages the connection
    synddb: Option<SyndDB>,
    /// When not replicated, we manage the connection directly
    standalone_conn: Option<&'static Connection>,
}

impl PredictionMarket {
    /// Create a new prediction market instance
    ///
    /// If `sequencer_url` is provided, uses `SyndDB::open()` which handles
    /// all the connection management internally.
    pub fn new(db_path: &str, sequencer_url: Option<&str>) -> Result<Self> {
        if let Some(url) = sequencer_url {
            // Use SyndDB::open() - it handles connection management internally
            let synddb = SyndDB::open(db_path, url)?;
            schema::initialize_schema(synddb.connection())?;
            Ok(Self {
                synddb: Some(synddb),
                standalone_conn: None,
            })
        } else {
            // No replication - manage connection ourselves
            let conn: &'static Connection = Box::leak(Box::new(Connection::open(db_path)?));
            schema::initialize_schema(conn)?;
            Ok(Self {
                synddb: None,
                standalone_conn: Some(conn),
            })
        }
    }

    /// Create an in-memory instance (for testing)
    pub fn in_memory(sequencer_url: Option<&str>) -> Result<Self> {
        if let Some(url) = sequencer_url {
            let synddb = SyndDB::open_in_memory(url)?;
            schema::initialize_schema(synddb.connection())?;
            Ok(Self {
                synddb: Some(synddb),
                standalone_conn: None,
            })
        } else {
            let conn: &'static Connection = Box::leak(Box::new(Connection::open_in_memory()?));
            schema::initialize_schema(conn)?;
            Ok(Self {
                synddb: None,
                standalone_conn: Some(conn),
            })
        }
    }

    /// Get a reference to the underlying connection
    pub fn conn(&self) -> &Connection {
        self.synddb.as_ref().map_or_else(
            || {
                self.standalone_conn
                    .expect("Either synddb or standalone_conn must be set")
            },
            |synddb| synddb.connection(),
        )
    }

    /// Check if `SyndDB` replication is enabled
    pub const fn is_replicated(&self) -> bool {
        self.synddb.is_some()
    }

    /// Explicitly publish pending changesets to the sequencer
    pub fn publish(&self) -> Result<()> {
        if let Some(ref synddb) = self.synddb {
            synddb.publish()?;
        }
        Ok(())
    }

    /// Get replication statistics (if replicated)
    pub fn stats(&self) -> Option<synddb_client::StatsSnapshot> {
        self.synddb.as_ref().map(|s| s.stats())
    }

    /// Check if sequencer is healthy (if replicated)
    pub fn is_healthy(&self) -> bool {
        self.synddb.as_ref().is_some_and(|s| s.is_healthy())
    }

    // =========================================================================
    // Account operations
    // =========================================================================

    /// Create a new account with default balance
    pub fn create_account(&self, name: &str) -> Result<i64> {
        self.conn().execute(
            "INSERT INTO accounts (name) VALUES (?1)",
            rusqlite::params![name],
        )?;
        Ok(self.conn().last_insert_rowid())
    }

    /// Get account by ID
    pub fn get_account(&self, account_id: i64) -> Result<Account> {
        self.conn()
            .query_row(
                "SELECT id, name, balance, created_at FROM accounts WHERE id = ?1",
                rusqlite::params![account_id],
                |row| {
                    Ok(Account {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        balance: row.get(2)?,
                        created_at: row.get(3)?,
                    })
                },
            )
            .map_err(Into::into)
    }

    /// Get account by name
    pub fn get_account_by_name(&self, name: &str) -> Result<Account> {
        self.conn()
            .query_row(
                "SELECT id, name, balance, created_at FROM accounts WHERE name = ?1",
                rusqlite::params![name],
                |row| {
                    Ok(Account {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        balance: row.get(2)?,
                        created_at: row.get(3)?,
                    })
                },
            )
            .map_err(Into::into)
    }

    /// List all accounts
    pub fn list_accounts(&self) -> Result<Vec<Account>> {
        let mut stmt = self
            .conn()
            .prepare("SELECT id, name, balance, created_at FROM accounts ORDER BY id")?;

        let accounts = stmt
            .query_map([], |row| {
                Ok(Account {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    balance: row.get(2)?,
                    created_at: row.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(accounts)
    }

    // =========================================================================
    // Market operations
    // =========================================================================

    /// Create a new prediction market
    pub fn create_market(
        &self,
        question: &str,
        description: Option<&str>,
        resolution_time: i64,
    ) -> Result<i64> {
        market::create_market(self.conn(), question, description, resolution_time)
    }

    /// Resolve a market with outcome "yes" or "no"
    pub fn resolve_market(&self, market_id: i64, outcome: &str) -> Result<()> {
        market::resolve_market(self.conn(), market_id, outcome)
    }

    /// Get market by ID
    pub fn get_market(&self, market_id: i64) -> Result<Market> {
        market::get_market(self.conn(), market_id)
    }

    /// List all markets
    pub fn list_markets(&self) -> Result<Vec<Market>> {
        market::list_markets(self.conn())
    }

    // =========================================================================
    // Trading operations
    // =========================================================================

    /// Buy shares in a market outcome
    pub fn buy_shares(
        &self,
        account_id: i64,
        market_id: i64,
        outcome: &str,
        shares: i64,
    ) -> Result<Trade> {
        trading::buy_shares(self.conn(), account_id, market_id, outcome, shares)
    }

    /// Sell shares in a market outcome
    pub fn sell_shares(
        &self,
        account_id: i64,
        market_id: i64,
        outcome: &str,
        shares: i64,
    ) -> Result<Trade> {
        trading::sell_shares(self.conn(), account_id, market_id, outcome, shares)
    }

    /// Get positions for an account
    pub fn get_positions(&self, account_id: i64) -> Result<Vec<Position>> {
        trading::get_positions(self.conn(), account_id)
    }

    // =========================================================================
    // Bridge operations (deposits/withdrawals)
    // =========================================================================

    /// Process pending deposits from chain monitor
    pub fn process_deposits(&self) -> Result<usize> {
        bridge::process_deposits(self.conn())
    }

    /// Request a withdrawal to L1
    pub fn request_withdrawal(
        &self,
        account_id: i64,
        amount: i64,
        destination_address: &str,
    ) -> Result<i64> {
        bridge::request_withdrawal(self.conn(), account_id, amount, destination_address)
    }

    /// List pending withdrawals
    pub fn list_pending_withdrawals(&self) -> Result<Vec<Withdrawal>> {
        bridge::list_pending_withdrawals(self.conn())
    }

    /// Simulate a deposit (for testing/demo)
    ///
    /// In production, the chain monitor would insert these records when it
    /// sees `Deposit` events from the bridge contract.
    pub fn simulate_deposit(
        &self,
        tx_hash: &str,
        from_address: &str,
        to_address: &str,
        amount: i64,
        block_number: i64,
    ) -> Result<i64> {
        bridge::simulate_deposit(
            self.conn(),
            tx_hash,
            from_address,
            to_address,
            amount,
            block_number,
        )
    }
}

#[derive(Debug, Clone)]
pub struct Account {
    pub id: i64,
    pub name: String,
    pub balance: i64,
    pub created_at: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_and_get_account() {
        let app = PredictionMarket::in_memory(None).unwrap();

        let id = app.create_account("alice").unwrap();
        let account = app.get_account(id).unwrap();

        assert_eq!(account.name, "alice");
        assert_eq!(account.balance, 1_000_000); // Default balance
    }

    #[test]
    fn test_full_trading_flow() {
        let app = PredictionMarket::in_memory(None).unwrap();

        // Create account and market
        let account_id = app.create_account("trader").unwrap();
        let market_id = app
            .create_market("Will BTC hit 100k?", None, 1800000000)
            .unwrap();

        // Buy YES shares
        let trade = app.buy_shares(account_id, market_id, "yes", 100).unwrap();
        assert_eq!(trade.shares, 100);
        assert_eq!(trade.total, 5000); // 100 * 50 cents

        // Check position
        let positions = app.get_positions(account_id).unwrap();
        assert_eq!(positions.len(), 1);
        assert_eq!(positions[0].shares, 100);

        // Resolve market as YES
        app.resolve_market(market_id, "yes").unwrap();

        // Check account got payout (starting 1M - 5000 cost + 10000 payout = 1005000)
        let account = app.get_account(account_id).unwrap();
        assert_eq!(account.balance, 1_005_000);
    }

    #[test]
    fn test_deposit_and_withdraw() {
        let app = PredictionMarket::in_memory(None).unwrap();

        // Simulate deposit for new account (to_address becomes account name)
        app.simulate_deposit("0xabc", "0xsender", "0xbob", 50000, 1000)
            .unwrap();
        let count = app.process_deposits().unwrap();
        assert_eq!(count, 1);

        // Check account was created using to_address as name
        let bob = app.get_account_by_name("0xbob").unwrap();
        assert_eq!(bob.balance, 50000);

        // Request withdrawal
        let w_id = app.request_withdrawal(bob.id, 20000, "0x1234").unwrap();
        assert!(w_id > 0);

        // Check balance was deducted
        let bob = app.get_account(bob.id).unwrap();
        assert_eq!(bob.balance, 30000);

        // Check withdrawal is pending
        let withdrawals = app.list_pending_withdrawals().unwrap();
        assert_eq!(withdrawals.len(), 1);
    }
}
