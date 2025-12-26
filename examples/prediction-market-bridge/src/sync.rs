//! Event sync from chain to local cache.
//!
//! This module demonstrates the complexity of keeping local state in sync
//! with on-chain state when using the message-passing paradigm.
//!
//! ## Challenges
//!
//! 1. **Eventual consistency**: Local cache always lags behind chain
//! 2. **Reorg handling**: Must handle chain reorganizations
//! 3. **Gap detection**: Must detect and fill missed events
//! 4. **Idempotency**: Must handle duplicate events gracefully

use anyhow::Result;
use rusqlite::{params, Connection};
use tracing::{debug, info, warn};

use crate::types::Outcome;

/// Event types emitted by the PredictionMarket contract.
#[derive(Debug, Clone)]
pub enum ContractEvent {
    MarketCreated {
        market_id: String,
        question: String,
        resolution_time: u64,
        block_number: u64,
        tx_hash: String,
    },
    MarketResolved {
        market_id: String,
        outcome: Outcome,
        block_number: u64,
        tx_hash: String,
    },
    Deposit {
        user: String,
        amount: u64,
        block_number: u64,
        tx_hash: String,
    },
    SharesPurchased {
        market_id: String,
        user: String,
        outcome: Outcome,
        shares: u64,
        block_number: u64,
        tx_hash: String,
        log_index: u64,
    },
    SharesSold {
        market_id: String,
        user: String,
        outcome: Outcome,
        shares: u64,
        block_number: u64,
        tx_hash: String,
        log_index: u64,
    },
}

/// Syncs events from the chain to the local cache.
#[derive(Debug)]
pub struct EventSyncer {
    /// Last synced block number.
    last_block: u64,
}

impl EventSyncer {
    /// Create a new syncer starting from the given block.
    pub const fn new(start_block: u64) -> Self {
        Self {
            last_block: start_block,
        }
    }

    /// Get last synced block from database.
    pub fn get_last_synced_block(conn: &Connection) -> Result<Option<u64>> {
        let result: Option<String> = conn
            .query_row(
                "SELECT value FROM sync_state WHERE key = 'last_block'",
                [],
                |row| row.get(0),
            )
            .ok();

        Ok(result.and_then(|v| v.parse().ok()))
    }

    /// Update last synced block in database.
    pub fn set_last_synced_block(conn: &Connection, block: u64) -> Result<()> {
        conn.execute(
            "INSERT OR REPLACE INTO sync_state (key, value) VALUES ('last_block', ?1)",
            params![block.to_string()],
        )?;
        Ok(())
    }

    /// Process a batch of events and update the local cache.
    ///
    /// This demonstrates the complexity of event processing:
    /// - Must be idempotent (same event can be processed multiple times)
    /// - Must handle events in order
    /// - Must update multiple tables atomically
    pub fn process_events(&mut self, conn: &Connection, events: Vec<ContractEvent>) -> Result<usize> {
        if events.is_empty() {
            return Ok(0);
        }

        let tx = conn.unchecked_transaction()?;
        let mut processed = 0;

        for event in events {
            match &event {
                ContractEvent::MarketCreated {
                    market_id,
                    question,
                    resolution_time,
                    block_number,
                    ..
                } => {
                    debug!(market_id, "Processing MarketCreated event");

                    // Insert market (idempotent via INSERT OR IGNORE)
                    tx.execute(
                        "INSERT OR IGNORE INTO markets
                         (id, question, resolution_time, resolved, created_at, created_block)
                         VALUES (?1, ?2, ?3, 0, ?4, ?5)",
                        params![
                            market_id,
                            question,
                            *resolution_time as i64,
                            *resolution_time as i64, // Use resolution_time as created_at placeholder
                            *block_number as i64,
                        ],
                    )?;

                    self.last_block = *block_number;
                    processed += 1;
                }

                ContractEvent::MarketResolved {
                    market_id,
                    outcome,
                    block_number,
                    ..
                } => {
                    debug!(market_id, ?outcome, "Processing MarketResolved event");

                    tx.execute(
                        "UPDATE markets
                         SET resolved = 1, winning_outcome = ?1, resolved_at = unixepoch(), resolved_block = ?2
                         WHERE id = ?3 AND resolved = 0",
                        params![outcome.as_u8() as i64, *block_number as i64, market_id],
                    )?;

                    self.last_block = *block_number;
                    processed += 1;
                }

                ContractEvent::Deposit {
                    user,
                    amount,
                    block_number,
                    ..
                } => {
                    debug!(user, amount, "Processing Deposit event");

                    // Create account if not exists, then add to balance
                    tx.execute(
                        "INSERT INTO accounts (address, balance, created_at)
                         VALUES (?1, ?2, unixepoch())
                         ON CONFLICT(address) DO UPDATE SET balance = balance + excluded.balance",
                        params![user, *amount as i64],
                    )?;

                    self.last_block = *block_number;
                    processed += 1;
                }

                ContractEvent::SharesPurchased {
                    market_id,
                    user,
                    outcome,
                    shares,
                    block_number,
                    tx_hash,
                    log_index,
                } => {
                    debug!(market_id, user, ?outcome, shares, "Processing SharesPurchased event");

                    let price = 50i64; // Fixed price
                    let total = (*shares as i64) * price;

                    // Check if already processed (idempotency via tx_hash + log_index)
                    let exists: bool = tx
                        .query_row(
                            "SELECT 1 FROM trades WHERE tx_hash = ?1 AND log_index = ?2",
                            params![tx_hash, *log_index as i64],
                            |_| Ok(true),
                        )
                        .unwrap_or(false);

                    if exists {
                        debug!(tx_hash, log_index, "Trade already processed, skipping");
                        continue;
                    }

                    // Update position
                    tx.execute(
                        "INSERT INTO positions (user, market_id, outcome, shares, cost_basis)
                         VALUES (?1, ?2, ?3, ?4, ?5)
                         ON CONFLICT(user, market_id, outcome) DO UPDATE SET
                           shares = shares + excluded.shares,
                           cost_basis = cost_basis + excluded.cost_basis",
                        params![user, market_id, outcome.as_u8() as i64, *shares as i64, total],
                    )?;

                    // Update market totals
                    let column = match outcome {
                        Outcome::Yes => "total_yes_shares",
                        Outcome::No => "total_no_shares",
                    };
                    tx.execute(
                        &format!(
                            "UPDATE markets SET {} = {} + ?1 WHERE id = ?2",
                            column, column
                        ),
                        params![*shares as i64, market_id],
                    )?;

                    // Deduct from account balance
                    tx.execute(
                        "UPDATE accounts SET balance = balance - ?1 WHERE address = ?2",
                        params![total, user],
                    )?;

                    // Record trade
                    tx.execute(
                        "INSERT INTO trades
                         (user, market_id, outcome, side, shares, price, total, executed_at, block_number, tx_hash, log_index)
                         VALUES (?1, ?2, ?3, 'buy', ?4, ?5, ?6, unixepoch(), ?7, ?8, ?9)",
                        params![
                            user,
                            market_id,
                            outcome.as_u8() as i64,
                            *shares as i64,
                            price,
                            total,
                            *block_number as i64,
                            tx_hash,
                            *log_index as i64
                        ],
                    )?;

                    self.last_block = *block_number;
                    processed += 1;
                }

                ContractEvent::SharesSold {
                    market_id,
                    user,
                    outcome,
                    shares,
                    block_number,
                    tx_hash,
                    log_index,
                } => {
                    debug!(market_id, user, ?outcome, shares, "Processing SharesSold event");

                    let price = 50i64;
                    let total = (*shares as i64) * price;

                    // Check if already processed
                    let exists: bool = tx
                        .query_row(
                            "SELECT 1 FROM trades WHERE tx_hash = ?1 AND log_index = ?2",
                            params![tx_hash, *log_index as i64],
                            |_| Ok(true),
                        )
                        .unwrap_or(false);

                    if exists {
                        continue;
                    }

                    // Update position
                    tx.execute(
                        "UPDATE positions SET shares = shares - ?1 WHERE user = ?2 AND market_id = ?3 AND outcome = ?4",
                        params![*shares as i64, user, market_id, outcome.as_u8() as i64],
                    )?;

                    // Update market totals
                    let column = match outcome {
                        Outcome::Yes => "total_yes_shares",
                        Outcome::No => "total_no_shares",
                    };
                    tx.execute(
                        &format!(
                            "UPDATE markets SET {} = {} - ?1 WHERE id = ?2",
                            column, column
                        ),
                        params![*shares as i64, market_id],
                    )?;

                    // Credit account balance
                    tx.execute(
                        "UPDATE accounts SET balance = balance + ?1 WHERE address = ?2",
                        params![total, user],
                    )?;

                    // Record trade
                    tx.execute(
                        "INSERT INTO trades
                         (user, market_id, outcome, side, shares, price, total, executed_at, block_number, tx_hash, log_index)
                         VALUES (?1, ?2, ?3, 'sell', ?4, ?5, ?6, unixepoch(), ?7, ?8, ?9)",
                        params![
                            user,
                            market_id,
                            outcome.as_u8() as i64,
                            *shares as i64,
                            price,
                            total,
                            *block_number as i64,
                            tx_hash,
                            *log_index as i64
                        ],
                    )?;

                    self.last_block = *block_number;
                    processed += 1;
                }
            }
        }

        // Update last synced block
        Self::set_last_synced_block(&tx, self.last_block)?;

        tx.commit()?;

        info!(processed, last_block = self.last_block, "Synced events to local cache");
        Ok(processed)
    }

    /// Handle a chain reorganization.
    ///
    /// This is one of the most complex aspects of event syncing - when the chain
    /// reorganizes, we need to roll back any events from the orphaned blocks.
    pub fn handle_reorg(&mut self, conn: &Connection, reorg_block: u64) -> Result<()> {
        warn!(reorg_block, "Handling chain reorganization");

        let tx = conn.unchecked_transaction()?;

        // Delete trades from reorged blocks (they'll be re-synced)
        tx.execute(
            "DELETE FROM trades WHERE block_number >= ?1",
            params![reorg_block as i64],
        )?;

        // Reset positions (would need to recalculate from remaining trades)
        // This is complex in practice - here we just warn
        warn!("Position recalculation after reorg not implemented");

        // Update sync state
        Self::set_last_synced_block(&tx, reorg_block.saturating_sub(1))?;
        self.last_block = reorg_block.saturating_sub(1);

        tx.commit()?;
        Ok(())
    }
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
    fn test_process_market_created() {
        let conn = setup();
        let mut syncer = EventSyncer::new(0);

        let events = vec![ContractEvent::MarketCreated {
            market_id: "0x1234".to_string(),
            question: "Will BTC hit 100k?".to_string(),
            resolution_time: 1800000000,
            block_number: 100,
            tx_hash: "0xabc".to_string(),
        }];

        let processed = syncer.process_events(&conn, events).unwrap();
        assert_eq!(processed, 1);

        // Verify market was created
        let question: String = conn
            .query_row(
                "SELECT question FROM markets WHERE id = '0x1234'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(question, "Will BTC hit 100k?");
    }

    #[test]
    fn test_process_deposit() {
        let conn = setup();
        let mut syncer = EventSyncer::new(0);

        let events = vec![ContractEvent::Deposit {
            user: "0xalice".to_string(),
            amount: 10000,
            block_number: 100,
            tx_hash: "0xabc".to_string(),
        }];

        syncer.process_events(&conn, events).unwrap();

        let balance: i64 = conn
            .query_row(
                "SELECT balance FROM accounts WHERE address = '0xalice'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(balance, 10000);
    }

    #[test]
    fn test_idempotent_trade_processing() {
        let conn = setup();
        let mut syncer = EventSyncer::new(0);

        // Setup: create market and account
        conn.execute(
            "INSERT INTO markets (id, question, resolution_time, resolved, created_at)
             VALUES ('0x1234', 'Test?', 1800000000, 0, 1700000000)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO accounts (address, balance) VALUES ('0xalice', 10000)",
            [],
        )
        .unwrap();

        let event = ContractEvent::SharesPurchased {
            market_id: "0x1234".to_string(),
            user: "0xalice".to_string(),
            outcome: Outcome::Yes,
            shares: 10,
            block_number: 100,
            tx_hash: "0xabc".to_string(),
            log_index: 0,
        };

        // Process same event twice
        syncer.process_events(&conn, vec![event.clone()]).unwrap();
        syncer.process_events(&conn, vec![event]).unwrap();

        // Should only have one trade
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM trades", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }
}
