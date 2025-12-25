use anyhow::{bail, Result};
use rusqlite::{params, Connection};

/// Process pending deposits from chain monitor
///
/// This function processes deposits that have been inserted into the `inbound_deposits`
/// table by the chain monitor. For each unprocessed deposit:
/// 1. Creates the account if it doesn't exist
/// 2. Credits the account balance
/// 3. Marks the deposit as processed
pub fn process_deposits(conn: &Connection) -> Result<usize> {
    let tx = conn.unchecked_transaction()?;

    // Get unprocessed deposits
    let deposits: Vec<(i64, String, i64)> = {
        let mut stmt = tx
            .prepare("SELECT id, account_name, amount FROM inbound_deposits WHERE processed = 0")?;
        let result: Vec<_> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        result
    };

    let count = deposits.len();

    for (deposit_id, account_name, amount) in deposits {
        // Create account if it doesn't exist, otherwise credit existing
        tx.execute(
            "INSERT INTO accounts (name, balance) VALUES (?1, ?2)
             ON CONFLICT(name) DO UPDATE SET balance = balance + excluded.balance",
            params![account_name, amount],
        )?;

        // Mark deposit as processed
        tx.execute(
            "UPDATE inbound_deposits SET processed = 1 WHERE id = ?1",
            params![deposit_id],
        )?;
    }

    tx.commit()?;
    Ok(count)
}

/// Request a withdrawal to L1
pub fn request_withdrawal(
    conn: &Connection,
    account_id: i64,
    amount: i64,
    destination_address: &str,
) -> Result<i64> {
    if amount <= 0 {
        bail!("withdrawal amount must be positive");
    }

    // Check account has sufficient balance
    let balance: i64 = conn.query_row(
        "SELECT balance FROM accounts WHERE id = ?1",
        params![account_id],
        |row| row.get(0),
    )?;

    if balance < amount {
        bail!(
            "insufficient balance: need {} cents, have {} cents",
            amount,
            balance
        );
    }

    let tx = conn.unchecked_transaction()?;

    // Deduct from balance
    tx.execute(
        "UPDATE accounts SET balance = balance - ?1 WHERE id = ?2",
        params![amount, account_id],
    )?;

    // Create withdrawal request
    tx.execute(
        "INSERT INTO outbound_withdrawals (account_id, amount, destination_address)
         VALUES (?1, ?2, ?3)",
        params![account_id, amount, destination_address],
    )?;

    let withdrawal_id = tx.last_insert_rowid();
    tx.commit()?;

    Ok(withdrawal_id)
}

/// List pending withdrawals
pub fn list_pending_withdrawals(conn: &Connection) -> Result<Vec<Withdrawal>> {
    let mut stmt = conn.prepare(
        "SELECT w.id, w.account_id, a.name, w.amount, w.destination_address, w.status, w.created_at
         FROM outbound_withdrawals w
         JOIN accounts a ON w.account_id = a.id
         WHERE w.status = 'pending'
         ORDER BY w.created_at ASC",
    )?;

    let withdrawals = stmt
        .query_map([], |row| {
            Ok(Withdrawal {
                id: row.get(0)?,
                account_id: row.get(1)?,
                account_name: row.get(2)?,
                amount: row.get(3)?,
                destination_address: row.get(4)?,
                status: row.get(5)?,
                created_at: row.get(6)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(withdrawals)
}

/// Simulate a deposit from L1 (for testing/demo purposes)
pub fn simulate_deposit(
    conn: &Connection,
    tx_hash: &str,
    account_name: &str,
    amount: i64,
    block_number: i64,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO inbound_deposits (tx_hash, account_name, amount, block_number)
         VALUES (?1, ?2, ?3, ?4)",
        params![tx_hash, account_name, amount, block_number],
    )?;
    Ok(conn.last_insert_rowid())
}

#[derive(Debug, Clone)]
pub struct Withdrawal {
    pub id: i64,
    pub account_id: i64,
    pub account_name: String,
    pub amount: i64,
    pub destination_address: String,
    pub status: String,
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
    fn test_process_deposits_new_account() {
        let conn = setup();

        // Simulate a deposit for a new account
        simulate_deposit(&conn, "0xabc123", "bob", 50000, 1000).unwrap();

        // Process deposits
        let count = process_deposits(&conn).unwrap();
        assert_eq!(count, 1);

        // Check account was created with deposit amount
        let balance: i64 = conn
            .query_row(
                "SELECT balance FROM accounts WHERE name = 'bob'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(balance, 50000);

        // Check deposit marked as processed
        let processed: i64 = conn
            .query_row(
                "SELECT processed FROM inbound_deposits WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(processed, 1);
    }

    #[test]
    fn test_process_deposits_existing_account() {
        let conn = setup();

        // Create existing account
        conn.execute(
            "INSERT INTO accounts (name, balance) VALUES ('alice', 100000)",
            [],
        )
        .unwrap();

        // Simulate a deposit
        simulate_deposit(&conn, "0xdef456", "alice", 25000, 1001).unwrap();

        process_deposits(&conn).unwrap();

        // Check balance was credited
        let balance: i64 = conn
            .query_row(
                "SELECT balance FROM accounts WHERE name = 'alice'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(balance, 125000);
    }

    #[test]
    fn test_request_withdrawal() {
        let conn = setup();

        // Create account
        conn.execute(
            "INSERT INTO accounts (name, balance) VALUES ('alice', 100000)",
            [],
        )
        .unwrap();

        // Request withdrawal
        let id = request_withdrawal(&conn, 1, 30000, "0x1234567890abcdef").unwrap();
        assert_eq!(id, 1);

        // Check balance was deducted
        let balance: i64 = conn
            .query_row("SELECT balance FROM accounts WHERE id = 1", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(balance, 70000);

        // Check withdrawal is pending
        let withdrawals = list_pending_withdrawals(&conn).unwrap();
        assert_eq!(withdrawals.len(), 1);
        assert_eq!(withdrawals[0].amount, 30000);
        assert_eq!(withdrawals[0].status, "pending");
    }

    #[test]
    fn test_withdrawal_insufficient_balance() {
        let conn = setup();

        conn.execute(
            "INSERT INTO accounts (name, balance) VALUES ('alice', 10000)",
            [],
        )
        .unwrap();

        let result = request_withdrawal(&conn, 1, 50000, "0x1234");
        assert!(result.is_err());
    }
}
