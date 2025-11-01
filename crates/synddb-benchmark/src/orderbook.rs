use anyhow::Result;
use rand::Rng;
use rusqlite::{params, Connection};
use std::time::{Duration, Instant};
use tracing::{debug, info};

use crate::load_patterns::{LoadConfig, LoadPattern};

const SYMBOLS: &[&str] = &["BTC-USD", "ETH-USD", "SOL-USD", "ARB-USD"];
const INITIAL_USERS: usize = 100;

pub struct OrderbookSimulator {
    conn: Connection,
    user_ids: Vec<i64>,
    operation_count: u64,
    start_time: Option<Instant>,
}

impl OrderbookSimulator {
    pub fn new(conn: Connection) -> Self {
        Self {
            conn,
            user_ids: Vec::new(),
            operation_count: 0,
            start_time: None,
        }
    }

    /// Run the simulation with the given load configuration
    pub async fn run(&mut self, config: LoadConfig) -> Result<()> {
        self.start_time = Some(Instant::now());

        // Initialize users if needed
        self.initialize_users()?;

        info!(
            "Starting simulation with {} users and pattern {:?}",
            self.user_ids.len(),
            config.pattern
        );

        let end_time = config
            .duration_seconds
            .map(|d| Instant::now() + Duration::from_secs(d));

        match config.pattern {
            LoadPattern::Continuous { ops_per_second } => {
                self.run_continuous(ops_per_second, end_time).await?;
            }
            LoadPattern::Burst {
                burst_size,
                pause_seconds,
            } => {
                self.run_burst(burst_size, pause_seconds, end_time).await?;
            }
        }

        Ok(())
    }

    async fn run_continuous(
        &mut self,
        ops_per_second: u64,
        end_time: Option<Instant>,
    ) -> Result<()> {
        let interval_micros = 1_000_000 / ops_per_second;
        let mut interval = tokio::time::interval(Duration::from_micros(interval_micros));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        let mut last_log = Instant::now();
        let log_interval = Duration::from_secs(5);

        loop {
            interval.tick().await;

            if let Some(end) = end_time {
                if Instant::now() >= end {
                    self.log_final_stats();
                    break;
                }
            }

            self.execute_random_operation()?;
            self.operation_count += 1;

            // Log stats periodically
            if last_log.elapsed() >= log_interval {
                self.log_stats();
                last_log = Instant::now();
            }
        }

        Ok(())
    }

    async fn run_burst(
        &mut self,
        burst_size: usize,
        pause_seconds: u64,
        end_time: Option<Instant>,
    ) -> Result<()> {
        let mut burst_num = 0;

        loop {
            if let Some(end) = end_time {
                if Instant::now() >= end {
                    self.log_final_stats();
                    break;
                }
            }

            burst_num += 1;
            info!("Executing burst #{} ({} operations)", burst_num, burst_size);

            let burst_start = Instant::now();

            for _ in 0..burst_size {
                self.execute_random_operation()?;
                self.operation_count += 1;
            }

            let burst_duration = burst_start.elapsed();
            info!(
                "Burst #{} completed in {:.2}s ({:.0} ops/sec)",
                burst_num,
                burst_duration.as_secs_f64(),
                burst_size as f64 / burst_duration.as_secs_f64()
            );

            self.log_stats();

            if end_time.is_none()
                || Instant::now() + Duration::from_secs(pause_seconds) < end_time.unwrap()
            {
                info!("Pausing for {} seconds...", pause_seconds);
                tokio::time::sleep(Duration::from_secs(pause_seconds)).await;
            }
        }

        Ok(())
    }

    fn initialize_users(&mut self) -> Result<()> {
        let existing_count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0))?;

        if existing_count == 0 {
            info!("Creating {} initial users...", INITIAL_USERS);

            let tx = self.conn.transaction()?;
            for i in 0..INITIAL_USERS {
                tx.execute(
                    "INSERT INTO users (username) VALUES (?1)",
                    params![format!("user_{}", i)],
                )?;
            }
            tx.commit()?;
        }

        // Load all user IDs
        let mut stmt = self.conn.prepare("SELECT id FROM users")?;
        let user_ids: Vec<i64> = stmt
            .query_map([], |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()?;

        self.user_ids = user_ids;

        // Initialize balances for all users
        for user_id in &self.user_ids {
            for symbol in SYMBOLS {
                self.conn.execute(
                    "INSERT OR IGNORE INTO balances (user_id, symbol, amount) VALUES (?1, ?2, ?3)",
                    params![user_id, symbol, 1_000_000_000],
                )?;
            }
        }

        info!("Initialized {} users with balances", self.user_ids.len());

        Ok(())
    }

    fn execute_random_operation(&mut self) -> Result<()> {
        let mut rng = rand::thread_rng();
        let operation_type = rng.gen_range(0..100);

        match operation_type {
            0..=50 => self.place_order()?,     // 51% - place order
            51..=65 => self.cancel_order()?,   // 15% - cancel order
            66..=85 => self.execute_trade()?,  // 20% - execute trade
            86..=99 => self.update_balance()?, // 14% - update balance
            _ => unreachable!(),
        }

        Ok(())
    }

    fn place_order(&mut self) -> Result<()> {
        let mut rng = rand::thread_rng();

        let user_id = self.user_ids[rng.gen_range(0..self.user_ids.len())];
        let symbol = SYMBOLS[rng.gen_range(0..SYMBOLS.len())];
        let side = if rng.gen_bool(0.5) { "buy" } else { "sell" };
        let order_type = "limit";
        let price = rng.gen_range(10_000..100_000);
        let quantity = rng.gen_range(1..100);

        self.conn.execute(
            "INSERT INTO orders (user_id, symbol, side, order_type, price, quantity, status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'active')",
            params![user_id, symbol, side, order_type, price, quantity],
        )?;

        debug!(
            "Placed {} order for {} {} at {}",
            side, quantity, symbol, price
        );

        Ok(())
    }

    fn cancel_order(&mut self) -> Result<()> {
        // Find a random active order
        let order_id: Option<i64> = self
            .conn
            .query_row(
                "SELECT id FROM orders WHERE status = 'active' ORDER BY RANDOM() LIMIT 1",
                [],
                |row| row.get(0),
            )
            .ok();

        if let Some(id) = order_id {
            self.conn.execute(
                "UPDATE orders SET status = 'cancelled', updated_at = unixepoch() WHERE id = ?1",
                params![id],
            )?;

            debug!("Cancelled order {}", id);
        }

        Ok(())
    }

    fn execute_trade(&mut self) -> Result<()> {
        let mut rng = rand::thread_rng();

        // Find a random buy and sell order
        let buy_order: Option<(i64, i64, String, i64)> = self
            .conn
            .query_row(
                "SELECT id, user_id, symbol, quantity FROM orders
             WHERE status = 'active' AND side = 'buy'
             ORDER BY RANDOM() LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .ok();

        let sell_order: Option<(i64, i64, String, i64)> = self
            .conn
            .query_row(
                "SELECT id, user_id, symbol, quantity FROM orders
             WHERE status = 'active' AND side = 'sell'
             ORDER BY RANDOM() LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .ok();

        if let (
            Some((buy_id, buyer_id, buy_symbol, buy_qty)),
            Some((sell_id, seller_id, sell_symbol, sell_qty)),
        ) = (buy_order, sell_order)
        {
            if buy_symbol == sell_symbol {
                let quantity = buy_qty.min(sell_qty);
                let price = rng.gen_range(10_000..100_000);

                // Create trade
                self.conn.execute(
                    "INSERT INTO trades (buy_order_id, sell_order_id, symbol, price, quantity, buyer_id, seller_id)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![buy_id, sell_id, buy_symbol, price, quantity, buyer_id, seller_id],
                )?;

                // Update orders
                self.conn.execute(
                    "UPDATE orders SET filled_quantity = filled_quantity + ?1,
                     status = CASE WHEN filled_quantity + ?1 >= quantity THEN 'filled' ELSE 'partial' END,
                     updated_at = unixepoch()
                     WHERE id = ?2",
                    params![quantity, buy_id],
                )?;

                self.conn.execute(
                    "UPDATE orders SET filled_quantity = filled_quantity + ?1,
                     status = CASE WHEN filled_quantity + ?1 >= quantity THEN 'filled' ELSE 'partial' END,
                     updated_at = unixepoch()
                     WHERE id = ?2",
                    params![quantity, sell_id],
                )?;

                debug!("Executed trade: {} {} at {}", quantity, buy_symbol, price);
            }
        }

        Ok(())
    }

    fn update_balance(&mut self) -> Result<()> {
        let mut rng = rand::thread_rng();

        let user_id = self.user_ids[rng.gen_range(0..self.user_ids.len())];
        let symbol = SYMBOLS[rng.gen_range(0..SYMBOLS.len())];
        let amount_change: i64 = rng.gen_range(-1000..1000);

        self.conn.execute(
            "UPDATE balances SET amount = MAX(0, amount + ?1), updated_at = unixepoch()
             WHERE user_id = ?2 AND symbol = ?3",
            params![amount_change, user_id, symbol],
        )?;

        debug!(
            "Updated balance for user {} symbol {} by {}",
            user_id, symbol, amount_change
        );

        Ok(())
    }

    fn log_stats(&self) {
        if let Some(start) = self.start_time {
            let elapsed = start.elapsed();
            let ops_per_sec = self.operation_count as f64 / elapsed.as_secs_f64();

            info!(
                "Operations: {} | Elapsed: {:.1}s | Rate: {:.1} ops/sec",
                self.operation_count,
                elapsed.as_secs_f64(),
                ops_per_sec
            );
        }
    }

    fn log_final_stats(&self) {
        info!("=== Simulation Complete ===");
        self.log_stats();

        if let Ok(stats) = self.get_db_stats() {
            info!("Final database state:");
            info!(
                "  Orders:    {} total ({} active)",
                stats.total_orders, stats.active_orders
            );
            info!("  Trades:    {}", stats.total_trades);
        }
    }

    fn get_db_stats(&self) -> Result<DbStats> {
        let total_orders: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM orders", [], |row| row.get(0))?;
        let active_orders: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM orders WHERE status = 'active'",
            [],
            |row| row.get(0),
        )?;
        let total_trades: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM trades", [], |row| row.get(0))?;

        Ok(DbStats {
            total_orders,
            active_orders,
            total_trades,
        })
    }
}

struct DbStats {
    total_orders: i64,
    active_orders: i64,
    total_trades: i64,
}
