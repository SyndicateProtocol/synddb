use anyhow::{bail, Result};
use rusqlite::{params, Connection};

/// Fixed price per share in cents (simplified pricing)
const PRICE_PER_SHARE: i64 = 50;

/// Buy shares in a market outcome
pub fn buy_shares(
    conn: &Connection,
    account_id: i64,
    market_id: i64,
    outcome: &str,
    shares: i64,
) -> Result<Trade> {
    if outcome != "yes" && outcome != "no" {
        bail!("outcome must be 'yes' or 'no'");
    }
    if shares <= 0 {
        bail!("shares must be positive");
    }

    // Check market is not resolved
    let market_outcome: String = conn.query_row(
        "SELECT outcome FROM markets WHERE id = ?1",
        params![market_id],
        |row| row.get(0),
    )?;

    if market_outcome != "unresolved" {
        bail!("market {} is already resolved", market_id);
    }

    let total_cost = shares * PRICE_PER_SHARE;

    // Check account has sufficient balance
    let balance: i64 = conn.query_row(
        "SELECT balance FROM accounts WHERE id = ?1",
        params![account_id],
        |row| row.get(0),
    )?;

    if balance < total_cost {
        bail!(
            "insufficient balance: need {} cents, have {} cents",
            total_cost,
            balance
        );
    }

    let tx = conn.unchecked_transaction()?;

    // Deduct from balance
    tx.execute(
        "UPDATE accounts SET balance = balance - ?1 WHERE id = ?2",
        params![total_cost, account_id],
    )?;

    // Update or insert position
    tx.execute(
        "INSERT INTO positions (account_id, market_id, outcome, shares, cost_basis)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(account_id, market_id, outcome) DO UPDATE SET
           shares = shares + excluded.shares,
           cost_basis = cost_basis + excluded.cost_basis",
        params![account_id, market_id, outcome, shares, total_cost],
    )?;

    // Record trade
    tx.execute(
        "INSERT INTO trades (account_id, market_id, outcome, side, shares, price, total)
         VALUES (?1, ?2, ?3, 'buy', ?4, ?5, ?6)",
        params![
            account_id,
            market_id,
            outcome,
            shares,
            PRICE_PER_SHARE,
            total_cost
        ],
    )?;

    let trade_id = tx.last_insert_rowid();
    tx.commit()?;

    Ok(Trade {
        id: trade_id,
        account_id,
        market_id,
        outcome: outcome.to_string(),
        side: "buy".to_string(),
        shares,
        price: PRICE_PER_SHARE,
        total: total_cost,
    })
}

/// Sell shares in a market outcome
pub fn sell_shares(
    conn: &Connection,
    account_id: i64,
    market_id: i64,
    outcome: &str,
    shares: i64,
) -> Result<Trade> {
    if outcome != "yes" && outcome != "no" {
        bail!("outcome must be 'yes' or 'no'");
    }
    if shares <= 0 {
        bail!("shares must be positive");
    }

    // Check market is not resolved
    let market_outcome: String = conn.query_row(
        "SELECT outcome FROM markets WHERE id = ?1",
        params![market_id],
        |row| row.get(0),
    )?;

    if market_outcome != "unresolved" {
        bail!("market {} is already resolved", market_id);
    }

    // Check position exists with enough shares
    let current_shares: i64 = conn
        .query_row(
            "SELECT shares FROM positions WHERE account_id = ?1 AND market_id = ?2 AND outcome = ?3",
            params![account_id, market_id, outcome],
            |row| row.get(0),
        )
        .unwrap_or(0);

    if current_shares < shares {
        bail!(
            "insufficient shares: trying to sell {}, have {}",
            shares,
            current_shares
        );
    }

    let total_proceeds = shares * PRICE_PER_SHARE;

    let tx = conn.unchecked_transaction()?;

    // Credit balance
    tx.execute(
        "UPDATE accounts SET balance = balance + ?1 WHERE id = ?2",
        params![total_proceeds, account_id],
    )?;

    // Update position
    tx.execute(
        "UPDATE positions SET shares = shares - ?1 WHERE account_id = ?2 AND market_id = ?3 AND outcome = ?4",
        params![shares, account_id, market_id, outcome],
    )?;

    // Record trade
    tx.execute(
        "INSERT INTO trades (account_id, market_id, outcome, side, shares, price, total)
         VALUES (?1, ?2, ?3, 'sell', ?4, ?5, ?6)",
        params![
            account_id,
            market_id,
            outcome,
            shares,
            PRICE_PER_SHARE,
            total_proceeds
        ],
    )?;

    let trade_id = tx.last_insert_rowid();
    tx.commit()?;

    Ok(Trade {
        id: trade_id,
        account_id,
        market_id,
        outcome: outcome.to_string(),
        side: "sell".to_string(),
        shares,
        price: PRICE_PER_SHARE,
        total: total_proceeds,
    })
}

/// Get positions for an account
pub fn get_positions(conn: &Connection, account_id: i64) -> Result<Vec<Position>> {
    let mut stmt = conn.prepare(
        "SELECT p.id, p.account_id, p.market_id, p.outcome, p.shares, p.cost_basis, m.question
         FROM positions p
         JOIN markets m ON p.market_id = m.id
         WHERE p.account_id = ?1 AND p.shares > 0",
    )?;

    let positions = stmt
        .query_map(params![account_id], |row| {
            Ok(Position {
                id: row.get(0)?,
                account_id: row.get(1)?,
                market_id: row.get(2)?,
                outcome: row.get(3)?,
                shares: row.get(4)?,
                cost_basis: row.get(5)?,
                market_question: row.get(6)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(positions)
}

#[derive(Debug, Clone)]
pub struct Trade {
    pub id: i64,
    pub account_id: i64,
    pub market_id: i64,
    pub outcome: String,
    pub side: String,
    pub shares: i64,
    pub price: i64,
    pub total: i64,
}

#[derive(Debug, Clone)]
pub struct Position {
    pub id: i64,
    pub account_id: i64,
    pub market_id: i64,
    pub outcome: String,
    pub shares: i64,
    pub cost_basis: i64,
    pub market_question: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{market::create_market, schema::initialize_schema};

    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        initialize_schema(&conn).unwrap();

        // Create a market and account
        create_market(&conn, "Test market?", None, 1700000000).unwrap();
        conn.execute(
            "INSERT INTO accounts (name, balance) VALUES ('alice', 100000)",
            [],
        )
        .unwrap();

        conn
    }

    #[test]
    fn test_buy_shares() {
        let conn = setup();

        let trade = buy_shares(&conn, 1, 1, "yes", 100).unwrap();
        assert_eq!(trade.shares, 100);
        assert_eq!(trade.total, 5000); // 100 * 50

        // Check balance was deducted
        let balance: i64 = conn
            .query_row("SELECT balance FROM accounts WHERE id = 1", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(balance, 95000); // 100000 - 5000

        // Check position was created
        let positions = get_positions(&conn, 1).unwrap();
        assert_eq!(positions.len(), 1);
        assert_eq!(positions[0].shares, 100);
    }

    #[test]
    fn test_buy_insufficient_balance() {
        let conn = setup();

        // Try to buy more than we can afford
        let result = buy_shares(&conn, 1, 1, "yes", 3000); // 3000 * 50 = 150000 > 100000
        assert!(result.is_err());
    }

    #[test]
    fn test_sell_shares() {
        let conn = setup();

        // Buy first
        buy_shares(&conn, 1, 1, "yes", 100).unwrap();

        // Then sell half
        let trade = sell_shares(&conn, 1, 1, "yes", 50).unwrap();
        assert_eq!(trade.shares, 50);
        assert_eq!(trade.total, 2500);

        // Check position updated
        let positions = get_positions(&conn, 1).unwrap();
        assert_eq!(positions[0].shares, 50);
    }

    #[test]
    fn test_sell_more_than_owned() {
        let conn = setup();

        buy_shares(&conn, 1, 1, "yes", 100).unwrap();

        let result = sell_shares(&conn, 1, 1, "yes", 200);
        assert!(result.is_err());
    }

    #[test]
    fn test_trade_on_resolved_market() {
        let conn = setup();

        // Resolve the market
        conn.execute("UPDATE markets SET outcome = 'yes' WHERE id = 1", [])
            .unwrap();

        let result = buy_shares(&conn, 1, 1, "yes", 10);
        assert!(result.is_err());
    }
}
