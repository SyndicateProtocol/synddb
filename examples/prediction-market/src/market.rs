use anyhow::{bail, Result};
use rusqlite::{params, Connection};

/// Create a new prediction market
pub fn create_market(
    conn: &Connection,
    question: &str,
    description: Option<&str>,
    resolution_time: i64,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO markets (question, description, resolution_time) VALUES (?1, ?2, ?3)",
        params![question, description, resolution_time],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Resolve a market with the given outcome
pub fn resolve_market(conn: &Connection, market_id: i64, outcome: &str) -> Result<()> {
    if outcome != "yes" && outcome != "no" {
        bail!("outcome must be 'yes' or 'no'");
    }

    // Check market exists and is unresolved
    let current_outcome: String = conn.query_row(
        "SELECT outcome FROM markets WHERE id = ?1",
        params![market_id],
        |row| row.get(0),
    )?;

    if current_outcome != "unresolved" {
        bail!(
            "market {} is already resolved as '{}'",
            market_id,
            current_outcome
        );
    }

    let tx = conn.unchecked_transaction()?;

    // Mark market as resolved
    tx.execute(
        "UPDATE markets SET outcome = ?1, resolved_at = unixepoch() WHERE id = ?2",
        params![outcome, market_id],
    )?;

    // Pay out winning positions: 100 cents per share
    let payout_per_share = 100;

    // Get all winning positions
    let winners: Vec<(i64, i64)> = {
        let mut stmt = tx.prepare(
            "SELECT account_id, shares FROM positions WHERE market_id = ?1 AND outcome = ?2 AND shares > 0",
        )?;
        let result: Vec<_> = stmt
            .query_map(params![market_id, outcome], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        result
    };

    // Credit each winner's account
    for (account_id, shares) in winners {
        let payout = shares * payout_per_share;
        tx.execute(
            "UPDATE accounts SET balance = balance + ?1 WHERE id = ?2",
            params![payout, account_id],
        )?;
    }

    tx.commit()?;
    Ok(())
}

/// Get market info
pub fn get_market(conn: &Connection, market_id: i64) -> Result<Market> {
    conn.query_row(
        "SELECT id, question, description, resolution_time, outcome, resolved_at, created_at
         FROM markets WHERE id = ?1",
        params![market_id],
        |row| {
            Ok(Market {
                id: row.get(0)?,
                question: row.get(1)?,
                description: row.get(2)?,
                resolution_time: row.get(3)?,
                outcome: row.get(4)?,
                resolved_at: row.get(5)?,
                created_at: row.get(6)?,
            })
        },
    )
    .map_err(Into::into)
}

/// List all markets
pub fn list_markets(conn: &Connection) -> Result<Vec<Market>> {
    let mut stmt = conn.prepare(
        "SELECT id, question, description, resolution_time, outcome, resolved_at, created_at
         FROM markets ORDER BY created_at DESC",
    )?;

    let markets = stmt
        .query_map([], |row| {
            Ok(Market {
                id: row.get(0)?,
                question: row.get(1)?,
                description: row.get(2)?,
                resolution_time: row.get(3)?,
                outcome: row.get(4)?,
                resolved_at: row.get(5)?,
                created_at: row.get(6)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(markets)
}

#[derive(Debug, Clone)]
pub struct Market {
    pub id: i64,
    pub question: String,
    pub description: Option<String>,
    pub resolution_time: i64,
    pub outcome: String,
    pub resolved_at: Option<i64>,
    pub created_at: i64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::initialize_schema;

    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        initialize_schema(&conn).unwrap();
        conn
    }

    #[test]
    fn test_create_market() {
        let conn = setup();
        let id = create_market(&conn, "Will it rain tomorrow?", None, 1700000000).unwrap();
        assert_eq!(id, 1);

        let market = get_market(&conn, id).unwrap();
        assert_eq!(market.question, "Will it rain tomorrow?");
        assert_eq!(market.outcome, "unresolved");
    }

    #[test]
    fn test_resolve_market() {
        let conn = setup();

        // Create market and account
        let market_id = create_market(&conn, "Test?", None, 1700000000).unwrap();
        conn.execute("INSERT INTO accounts (name) VALUES ('alice')", [])
            .unwrap();

        // Give alice some YES shares
        conn.execute(
            "INSERT INTO positions (account_id, market_id, outcome, shares, cost_basis)
             VALUES (1, ?1, 'yes', 10, 500)",
            params![market_id],
        )
        .unwrap();

        // Resolve as YES
        resolve_market(&conn, market_id, "yes").unwrap();

        // Check market is resolved
        let market = get_market(&conn, market_id).unwrap();
        assert_eq!(market.outcome, "yes");

        // Check alice got paid (starting 1000000 + 10 shares * 100 = 1001000)
        let balance: i64 = conn
            .query_row("SELECT balance FROM accounts WHERE id = 1", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(balance, 1001000);
    }

    #[test]
    fn test_resolve_already_resolved() {
        let conn = setup();
        let market_id = create_market(&conn, "Test?", None, 1700000000).unwrap();
        resolve_market(&conn, market_id, "yes").unwrap();

        let result = resolve_market(&conn, market_id, "no");
        assert!(result.is_err());
    }
}
