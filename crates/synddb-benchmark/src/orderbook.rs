use anyhow::Result;
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rand::Rng;
use rusqlite::params;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tracing::info;

use crate::load_patterns::{LoadConfig, LoadPattern};

const SYMBOLS: &[&str] = &["BTC-USD", "ETH-USD", "SOL-USD", "ARB-USD"];
const INITIAL_USERS: usize = 100;

pub struct OrderbookSimulator {
    pool: Pool<SqliteConnectionManager>,
    user_ids: Vec<i64>,
    operation_count: Arc<AtomicU64>,
    start_time: Option<Instant>,
}

impl OrderbookSimulator {
    pub fn new(pool: Pool<SqliteConnectionManager>) -> Self {
        Self {
            pool,
            user_ids: Vec::new(),
            operation_count: Arc::new(AtomicU64::new(0)),
            start_time: None,
        }
    }

    /// Run the simulation with the given load configuration
    pub async fn run(&mut self, config: LoadConfig) -> Result<()> {
        self.start_time = Some(Instant::now());

        // Initialize users if needed
        self.initialize_users()?;

        let mode = if config.simple_mode {
            "SIMPLE (inserts only)"
        } else {
            "FULL (all operations)"
        };

        info!(
            "Starting simulation with {} users and pattern {:?} in {} mode",
            self.user_ids.len(),
            config.pattern,
            mode
        );

        let end_time = config
            .duration_seconds
            .map(|d| Instant::now() + Duration::from_secs(d));

        // Use parallel workers for better CPU utilization
        if config.num_workers > 1 {
            info!(
                "Running with {} parallel workers for maximum throughput",
                config.num_workers
            );
            match config.pattern {
                LoadPattern::Continuous { ops_per_second } => {
                    self.run_continuous_parallel(
                        ops_per_second,
                        end_time,
                        config.batch_size,
                        config.simple_mode,
                        config.num_workers,
                    )
                    .await?;
                }
                LoadPattern::MaxThroughput => {
                    self.run_max_throughput_parallel(
                        end_time,
                        config.batch_size,
                        config.simple_mode,
                        config.num_workers,
                    )
                    .await?;
                }
                LoadPattern::Burst {
                    burst_size,
                    pause_seconds,
                } => {
                    self.run_burst(
                        burst_size,
                        pause_seconds,
                        end_time,
                        config.batch_size,
                        config.simple_mode,
                    )
                    .await?;
                }
            }
        } else {
            // Single-threaded execution (legacy)
            match config.pattern {
                LoadPattern::Continuous { ops_per_second } => {
                    self.run_continuous(
                        ops_per_second,
                        end_time,
                        config.batch_size,
                        config.simple_mode,
                    )
                    .await?;
                }
                LoadPattern::Burst {
                    burst_size,
                    pause_seconds,
                } => {
                    self.run_burst(
                        burst_size,
                        pause_seconds,
                        end_time,
                        config.batch_size,
                        config.simple_mode,
                    )
                    .await?;
                }
                LoadPattern::MaxThroughput => {
                    self.run_max_throughput(end_time, config.batch_size, config.simple_mode)
                        .await?;
                }
            }
        }

        Ok(())
    }

    async fn run_continuous(
        &mut self,
        ops_per_second: u64,
        end_time: Option<Instant>,
        batch_size: usize,
        simple_mode: bool,
    ) -> Result<()> {
        let interval_micros = 1_000_000 / ops_per_second;
        let mut interval =
            tokio::time::interval(Duration::from_micros(interval_micros * batch_size as u64));
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

            // Execute a batch of operations in a single transaction
            {
                let mut conn = self.pool.get()?;
                let tx = conn.transaction()?;
                for _ in 0..batch_size {
                    if let Some(end) = end_time {
                        if Instant::now() >= end {
                            break;
                        }
                    }
                    if simple_mode {
                        Self::execute_simple_operation_in_tx_static(&self.user_ids, &tx)?;
                    } else {
                        Self::execute_random_operation_in_tx_static(&self.user_ids, &tx)?;
                    }
                    self.operation_count.fetch_add(1, Ordering::Relaxed);
                }
                tx.commit()?;
            }

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
        batch_size: usize,
        simple_mode: bool,
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

            // Execute burst in batches
            let num_batches = (burst_size + batch_size - 1) / batch_size;
            for batch_idx in 0..num_batches {
                let batch_ops = if batch_idx == num_batches - 1 {
                    burst_size - (batch_idx * batch_size)
                } else {
                    batch_size
                };

                let mut conn = self.pool.get()?;
                let tx = conn.transaction()?;
                for _ in 0..batch_ops {
                    if simple_mode {
                        Self::execute_simple_operation_in_tx_static(&self.user_ids, &tx)?;
                    } else {
                        Self::execute_random_operation_in_tx_static(&self.user_ids, &tx)?;
                    }
                    self.operation_count.fetch_add(1, Ordering::Relaxed);
                }
                tx.commit()?;
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

    /// Run continuous load with parallel workers for maximum CPU utilization
    async fn run_continuous_parallel(
        &mut self,
        ops_per_second: u64,
        end_time: Option<Instant>,
        batch_size: usize,
        simple_mode: bool,
        num_workers: usize,
    ) -> Result<()> {
        let (tx, rx) = mpsc::channel::<()>(num_workers * 2);

        // Spawn worker tasks - each gets work from broadcast channel
        let mut handles = vec![];
        let work_rx_arc = Arc::new(tokio::sync::Mutex::new(rx));

        for _worker_id in 0..num_workers {
            let pool = self.pool.clone();
            let user_ids = self.user_ids.clone();
            let worker_rx = work_rx_arc.clone();
            let counter = self.operation_count.clone();

            let handle = tokio::spawn(async move {
                loop {
                    let msg = {
                        let mut rx = worker_rx.lock().await;
                        rx.recv().await
                    };

                    if msg.is_none() {
                        break;
                    }

                    let mut conn = pool.get().map_err(|e| anyhow::anyhow!("{}", e))?;
                    let tx = conn.transaction()?;

                    for _ in 0..batch_size {
                        if simple_mode {
                            Self::execute_simple_operation_in_tx_static(&user_ids, &tx)?;
                        } else {
                            Self::execute_random_operation_in_tx_static(&user_ids, &tx)?;
                        }
                        counter.fetch_add(1, Ordering::Relaxed);
                    }
                    tx.commit()?;
                }
                Ok::<_, anyhow::Error>(())
            });

            handles.push(handle);
        }

        // Main loop: dispatch work to workers
        let interval_micros = (1_000_000 / ops_per_second) * batch_size as u64;
        let mut interval = tokio::time::interval(Duration::from_micros(interval_micros));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        let mut last_log = Instant::now();
        let log_interval = Duration::from_secs(5);

        loop {
            interval.tick().await;

            if let Some(end) = end_time {
                if Instant::now() >= end {
                    break;
                }
            }

            // Send work signal (workers process independently)
            if tx.send(()).await.is_err() {
                break;
            }

            // Log stats periodically
            if last_log.elapsed() >= log_interval {
                self.log_stats();
                last_log = Instant::now();
            }
        }

        // Shutdown workers
        drop(tx);
        for handle in handles {
            handle.await??;
        }

        self.log_final_stats();
        Ok(())
    }

    /// Run max throughput discovery with parallel workers
    async fn run_max_throughput_parallel(
        &mut self,
        end_time: Option<Instant>,
        batch_size: usize,
        simple_mode: bool,
        num_workers: usize,
    ) -> Result<()> {
        info!("=== Max Throughput Discovery Mode ===");
        info!("Will automatically find maximum sustainable throughput");
        info!("Using adaptive algorithm with stability detection");

        // Spawn persistent worker pool
        let (work_tx, work_rx) = mpsc::channel::<()>(num_workers * 4);
        let mut handles = vec![];
        let work_rx_arc = Arc::new(tokio::sync::Mutex::new(work_rx));

        for _ in 0..num_workers {
            let pool = self.pool.clone();
            let user_ids = self.user_ids.clone();
            let worker_rx = work_rx_arc.clone();
            let counter = self.operation_count.clone();

            let handle = tokio::spawn(async move {
                loop {
                    let msg = {
                        let mut rx = worker_rx.lock().await;
                        rx.recv().await
                    };

                    if msg.is_none() {
                        break;
                    }

                    let mut conn = pool.get().map_err(|e| anyhow::anyhow!("{}", e))?;
                    let tx = conn.transaction()?;

                    for _ in 0..batch_size {
                        if simple_mode {
                            Self::execute_simple_operation_in_tx_static(&user_ids, &tx)?;
                        } else {
                            Self::execute_random_operation_in_tx_static(&user_ids, &tx)?;
                        }
                        counter.fetch_add(1, Ordering::Relaxed);
                    }
                    tx.commit()?;
                }
                Ok::<_, anyhow::Error>(())
            });

            handles.push(handle);
        }

        // Test increasing rates
        let mut current_rate = 1000;
        let mut best_stable_rate = 0.0;

        loop {
            if let Some(end) = end_time {
                if Instant::now() >= end {
                    break;
                }
            }

            info!("\n--- Testing {} ops/sec ---", current_rate);

            let sample_rate = self
                .test_rate_parallel(current_rate, batch_size, &work_tx, num_workers)
                .await?;

            let achievement = (sample_rate / current_rate as f64) * 100.0;

            // Check if we achieved target
            if achievement >= 90.0 {
                best_stable_rate = sample_rate;
                current_rate *= 2; // Double the rate
            } else {
                info!("⚠ Throughput degraded (<90% of target) - backing off");
                break;
            }
        }

        info!(
            "\n=== Maximum Throughput Found ===\nBest sustained rate: {:.0} ops/sec\n",
            best_stable_rate
        );

        // Shutdown workers
        drop(work_tx);
        for handle in handles {
            handle.await??;
        }

        self.log_final_stats();
        Ok(())
    }

    /// Test a specific rate with parallel workers
    async fn test_rate_parallel(
        &mut self,
        rate: u64,
        batch_size: usize,
        work_tx: &mpsc::Sender<()>,
        num_workers: usize,
    ) -> Result<f64> {
        let num_samples = 3;
        let sample_duration = Duration::from_secs(3);
        let mut sample_rates = Vec::new();

        for sample_num in 1..=num_samples {
            let sample_start = Instant::now();
            let ops_before = self.operation_count.load(Ordering::Relaxed);
            let sample_end_time = sample_start + sample_duration;

            let interval_micros = (1_000_000 / rate) * batch_size as u64;
            let mut interval = tokio::time::interval(Duration::from_micros(interval_micros));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

            while Instant::now() < sample_end_time {
                interval.tick().await;
                // Send work to all workers
                for _ in 0..num_workers {
                    if work_tx.send(()).await.is_err() {
                        break;
                    }
                }
            }

            let sample_elapsed = sample_start.elapsed();
            let ops_completed = self.operation_count.load(Ordering::Relaxed) - ops_before;
            let sample_rate = ops_completed as f64 / sample_elapsed.as_secs_f64();
            sample_rates.push(sample_rate);

            info!(
                "  Sample {}/{}: {:.0} ops/sec",
                sample_num, num_samples, sample_rate
            );
        }

        let mean_rate = sample_rates.iter().sum::<f64>() / sample_rates.len() as f64;
        let std_dev = (sample_rates
            .iter()
            .map(|r| (r - mean_rate).powi(2))
            .sum::<f64>()
            / sample_rates.len() as f64)
            .sqrt();
        let cv = (std_dev / mean_rate) * 100.0;

        info!(
            "Mean: {:.0} ops/sec ({:.1}% of target) | Stability: {:.1}% CV",
            mean_rate,
            (mean_rate / rate as f64) * 100.0,
            cv
        );

        Ok(mean_rate)
    }

    async fn run_max_throughput(
        &mut self,
        end_time: Option<Instant>,
        batch_size: usize,
        simple_mode: bool,
    ) -> Result<()> {
        info!("=== Max Throughput Discovery Mode ===");
        info!("Will automatically find maximum sustainable throughput");
        info!("Using adaptive algorithm with stability detection");

        let mut current_rate = 1_000u64;
        let mut best_rate = 0u64;
        let mut best_actual_rate = 0f64;
        let mut best_stability = 0f64;

        loop {
            if let Some(end) = end_time {
                if Instant::now() >= end {
                    break;
                }
            }

            info!("\n--- Testing {} ops/sec ---", current_rate);

            // Run multiple samples to measure stability
            let num_samples = 3;
            let sample_duration = Duration::from_secs(3);
            let mut sample_rates = Vec::new();

            for sample_num in 1..=num_samples {
                let sample_start = Instant::now();
                let ops_before = self.operation_count.load(Ordering::Relaxed);
                let sample_end_time = sample_start + sample_duration;

                let interval_micros = 1_000_000 / current_rate;
                let mut interval = tokio::time::interval(Duration::from_micros(
                    interval_micros * batch_size as u64,
                ));
                interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

                while Instant::now() < sample_end_time {
                    interval.tick().await;

                    let mut conn = self.pool.get()?;
                    let tx = conn.transaction()?;
                    for _ in 0..batch_size {
                        if simple_mode {
                            Self::execute_simple_operation_in_tx_static(&self.user_ids, &tx)?;
                        } else {
                            Self::execute_random_operation_in_tx_static(&self.user_ids, &tx)?;
                        }
                        self.operation_count.fetch_add(1, Ordering::Relaxed);
                    }
                    tx.commit()?;
                }

                let sample_elapsed = sample_start.elapsed();
                let ops_completed = self.operation_count.load(Ordering::Relaxed) - ops_before;
                let sample_rate = ops_completed as f64 / sample_elapsed.as_secs_f64();
                sample_rates.push(sample_rate);

                info!(
                    "  Sample {}/{}: {:.0} ops/sec",
                    sample_num, num_samples, sample_rate
                );
            }

            // Calculate statistics
            let mean_rate = sample_rates.iter().sum::<f64>() / sample_rates.len() as f64;
            let variance = sample_rates
                .iter()
                .map(|r| (r - mean_rate).powi(2))
                .sum::<f64>()
                / sample_rates.len() as f64;
            let std_dev = variance.sqrt();
            let coefficient_of_variation = std_dev / mean_rate; // Lower = more stable
            let achievement_pct = (mean_rate / current_rate as f64) * 100.0;

            info!(
                "Mean: {:.0} ops/sec ({:.1}% of target) | Stability: {:.1}% CV",
                mean_rate,
                achievement_pct,
                coefficient_of_variation * 100.0
            );

            // Define degradation criteria
            let is_degraded = achievement_pct < 90.0; // Not hitting target
            let is_unstable = coefficient_of_variation > 0.15; // >15% variance
            let is_marginal = achievement_pct < 95.0;

            if is_degraded {
                info!("⚠ Throughput degraded (<90% of target) - backing off");

                // Back off by 10% and verify stability
                let backoff_rate = (current_rate as f64 * 0.9) as u64;
                info!("\n--- Verifying stability at {} ops/sec ---", backoff_rate);

                let verify_mean = self
                    .run_verification_test(backoff_rate, batch_size, simple_mode)
                    .await?;

                info!(
                    "\n=== Maximum Throughput Found ===\n\
                     Best sustained rate: {:.0} ops/sec\n\
                     Verified stable rate: {:.0} ops/sec\n\
                     Degradation detected at: {} ops/sec target\n",
                    best_actual_rate, verify_mean, current_rate
                );
                break;
            }

            if is_unstable {
                info!(
                    "⚠ Performance unstable (CV {:.1}%) - system under stress",
                    coefficient_of_variation * 100.0
                );

                // If this is worse stability than our best, we've found the limit
                if best_rate > 0 && coefficient_of_variation > best_stability * 1.5 {
                    info!(
                        "\n=== Maximum Stable Throughput Found ===\n\
                         Best sustained rate: {:.0} ops/sec (CV {:.1}%)\n\
                         Current rate unstable: {:.0} ops/sec (CV {:.1}%)\n\
                         Recommendation: Use {} ops/sec for stable operation\n",
                        best_actual_rate,
                        best_stability * 100.0,
                        mean_rate,
                        coefficient_of_variation * 100.0,
                        best_rate
                    );
                    break;
                }
            }

            // Update best rate if this is better
            if mean_rate > best_actual_rate && !is_unstable {
                best_rate = current_rate;
                best_actual_rate = mean_rate;
                best_stability = coefficient_of_variation;
            }

            // Determine next rate
            let next_rate = if is_marginal || is_unstable {
                // Near limit or unstable - use smaller increments
                ((current_rate as f64 * 1.2) as u64).max(current_rate + 100)
            } else {
                // Far from limit - double it
                current_rate * 2
            };

            // Cap at reasonable maximum to avoid overflow
            if next_rate > 10_000_000 {
                info!(
                    "\n=== Reached Testing Limit ===\n\
                     Maximum tested rate: {:.0} ops/sec\n\
                     Stability: {:.1}% CV\n\
                     System can sustain even higher throughput\n",
                    best_actual_rate,
                    best_stability * 100.0
                );
                break;
            }

            current_rate = next_rate;
        }

        self.log_final_stats();
        Ok(())
    }

    async fn run_verification_test(
        &mut self,
        rate: u64,
        batch_size: usize,
        simple_mode: bool,
    ) -> Result<f64> {
        let num_samples = 3;
        let sample_duration = Duration::from_secs(3);
        let mut sample_rates = Vec::new();

        for sample_num in 1..=num_samples {
            let sample_start = Instant::now();
            let ops_before = self.operation_count.load(Ordering::Relaxed);
            let sample_end_time = sample_start + sample_duration;

            let interval_micros = 1_000_000 / rate;
            let mut interval =
                tokio::time::interval(Duration::from_micros(interval_micros * batch_size as u64));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

            while Instant::now() < sample_end_time {
                interval.tick().await;

                let mut conn = self.pool.get()?;
                let tx = conn.transaction()?;
                for _ in 0..batch_size {
                    if simple_mode {
                        Self::execute_simple_operation_in_tx_static(&self.user_ids, &tx)?;
                    } else {
                        Self::execute_random_operation_in_tx_static(&self.user_ids, &tx)?;
                    }
                    self.operation_count.fetch_add(1, Ordering::Relaxed);
                }
                tx.commit()?;
            }

            let sample_elapsed = sample_start.elapsed();
            let ops_completed = self.operation_count.load(Ordering::Relaxed) - ops_before;
            let sample_rate = ops_completed as f64 / sample_elapsed.as_secs_f64();
            sample_rates.push(sample_rate);

            info!(
                "  Verification {}/{}: {:.0} ops/sec",
                sample_num, num_samples, sample_rate
            );
        }

        Ok(sample_rates.iter().sum::<f64>() / sample_rates.len() as f64)
    }

    fn initialize_users(&mut self) -> Result<()> {
        let conn = self.pool.get()?;
        let existing_count: i64 =
            conn.query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0))?;

        if existing_count == 0 {
            info!("Creating {} initial users...", INITIAL_USERS);

            let mut conn = self.pool.get()?;
            let tx = conn.transaction()?;
            for i in 0..INITIAL_USERS {
                tx.execute(
                    "INSERT INTO users (username) VALUES (?1)",
                    params![format!("user_{}", i)],
                )?;
            }
            tx.commit()?;
        }

        // Load all user IDs
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare("SELECT id FROM users")?;
        let user_ids: Vec<i64> = stmt
            .query_map([], |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()?;

        self.user_ids = user_ids;

        // Initialize balances for all users
        for user_id in &self.user_ids {
            for symbol in SYMBOLS {
                conn.execute(
                    "INSERT OR IGNORE INTO balances (user_id, symbol, amount) VALUES (?1, ?2, ?3)",
                    params![user_id, symbol, 1_000_000_000],
                )?;
            }
        }

        info!("Initialized {} users with balances", self.user_ids.len());

        Ok(())
    }

    fn execute_random_operation_in_tx_static(
        user_ids: &[i64],
        tx: &rusqlite::Transaction,
    ) -> Result<()> {
        let mut rng = rand::thread_rng();
        let operation_type = rng.gen_range(0..100);

        match operation_type {
            0..=50 => Self::place_order_in_tx(user_ids, tx)?, // 51% - place order
            51..=65 => Self::cancel_order_in_tx(tx)?,         // 15% - cancel order
            66..=85 => Self::execute_trade_in_tx(tx)?,        // 20% - execute trade
            86..=99 => Self::update_balance_in_tx(user_ids, tx)?, // 14% - update balance
            _ => unreachable!(),
        }

        Ok(())
    }

    /// Execute a simple operation (just insert an order) - for maximum throughput testing
    /// This bypasses all the complex queries (ORDER BY RANDOM(), joins, etc.)
    fn execute_simple_operation_in_tx_static(
        user_ids: &[i64],
        tx: &rusqlite::Transaction,
    ) -> Result<()> {
        let mut rng = rand::thread_rng();

        let user_id = user_ids[rng.gen_range(0..user_ids.len())];
        let symbol = SYMBOLS[rng.gen_range(0..SYMBOLS.len())];
        let side = if rng.gen_bool(0.5) { "buy" } else { "sell" };
        let price = rng.gen_range(10_000..100_000);
        let quantity = rng.gen_range(1..100);

        tx.execute(
            "INSERT INTO orders (user_id, symbol, side, order_type, price, quantity, status)
             VALUES (?1, ?2, ?3, 'limit', ?4, ?5, 'active')",
            params![user_id, symbol, side, price, quantity],
        )?;

        Ok(())
    }

    fn log_stats(&self) {
        if let Some(start) = self.start_time {
            let elapsed = start.elapsed();
            let total_ops = self.operation_count.load(Ordering::Relaxed);
            let ops_per_sec = total_ops as f64 / elapsed.as_secs_f64();

            info!(
                "Operations: {} | Elapsed: {:.1}s | Rate: {:.1} ops/sec",
                total_ops,
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
        let conn = self.pool.get()?;
        let total_orders: i64 =
            conn.query_row("SELECT COUNT(*) FROM orders", [], |row| row.get(0))?;
        let active_orders: i64 = conn.query_row(
            "SELECT COUNT(*) FROM orders WHERE status = 'active'",
            [],
            |row| row.get(0),
        )?;
        let total_trades: i64 =
            conn.query_row("SELECT COUNT(*) FROM orders", [], |row| row.get(0))?;

        Ok(DbStats {
            total_orders,
            active_orders,
            total_trades,
        })
    }

    // Transaction-based versions for batching
    fn place_order_in_tx(user_ids: &[i64], tx: &rusqlite::Transaction) -> Result<()> {
        let mut rng = rand::thread_rng();
        let user_id = user_ids[rng.gen_range(0..user_ids.len())];
        let symbol = SYMBOLS[rng.gen_range(0..SYMBOLS.len())];
        let side = if rng.gen_bool(0.5) { "buy" } else { "sell" };
        let price = rng.gen_range(10_000..100_000);
        let quantity = rng.gen_range(1..100);

        tx.execute(
            "INSERT INTO orders (user_id, symbol, side, order_type, price, quantity, status)
             VALUES (?1, ?2, ?3, 'limit', ?4, ?5, 'active')",
            params![user_id, symbol, side, price, quantity],
        )?;
        Ok(())
    }

    fn cancel_order_in_tx(tx: &rusqlite::Transaction) -> Result<()> {
        // Efficient random selection using OFFSET - much faster than ORDER BY RANDOM() and MIN/MAX
        // This approach uses an index-only scan with OFFSET for O(log n) performance
        let count: Result<i64, _> = tx.query_row(
            "SELECT COUNT(*) FROM orders WHERE status = 'active'",
            [],
            |row| row.get(0),
        );

        if let Ok(total) = count {
            if total > 0 {
                let random_offset = rand::thread_rng().gen_range(0..total);

                let order_id: Option<i64> = tx
                    .query_row(
                        "SELECT id FROM orders WHERE status = 'active' LIMIT 1 OFFSET ?1",
                        params![random_offset],
                        |row| row.get(0),
                    )
                    .ok();

                if let Some(id) = order_id {
                    tx.execute(
                        "UPDATE orders SET status = 'cancelled', updated_at = unixepoch() WHERE id = ?1",
                        params![id],
                    )?;
                }
            }
        }
        Ok(())
    }

    fn execute_trade_in_tx(tx: &rusqlite::Transaction) -> Result<()> {
        let mut rng = rand::thread_rng();

        // Efficient random selection for buy orders using OFFSET
        let buy_order: Option<(i64, i64, String, i64)> = {
            let count: Result<i64, _> = tx.query_row(
                "SELECT COUNT(*) FROM orders WHERE status = 'active' AND side = 'buy'",
                [],
                |row| row.get(0),
            );

            if let Ok(total) = count {
                if total > 0 {
                    let random_offset = rng.gen_range(0..total);
                    tx.query_row(
                        "SELECT id, user_id, symbol, quantity FROM orders
                         WHERE status = 'active' AND side = 'buy' LIMIT 1 OFFSET ?1",
                        params![random_offset],
                        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                    )
                    .ok()
                } else {
                    None
                }
            } else {
                None
            }
        };

        // Efficient random selection for sell orders using OFFSET
        let sell_order: Option<(i64, i64, String, i64)> = {
            let count: Result<i64, _> = tx.query_row(
                "SELECT COUNT(*) FROM orders WHERE status = 'active' AND side = 'sell'",
                [],
                |row| row.get(0),
            );

            if let Ok(total) = count {
                if total > 0 {
                    let random_offset = rng.gen_range(0..total);
                    tx.query_row(
                        "SELECT id, user_id, symbol, quantity FROM orders
                         WHERE status = 'active' AND side = 'sell' LIMIT 1 OFFSET ?1",
                        params![random_offset],
                        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                    )
                    .ok()
                } else {
                    None
                }
            } else {
                None
            }
        };

        if let (
            Some((buy_id, buyer_id, buy_symbol, buy_qty)),
            Some((sell_id, seller_id, sell_symbol, sell_qty)),
        ) = (buy_order, sell_order)
        {
            if buy_symbol == sell_symbol {
                let quantity = buy_qty.min(sell_qty);
                let price = rng.gen_range(10_000..100_000);

                tx.execute(
                    "INSERT INTO trades (buy_order_id, sell_order_id, symbol, price, quantity, buyer_id, seller_id)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![buy_id, sell_id, buy_symbol, price, quantity, buyer_id, seller_id],
                )?;

                tx.execute(
                    "UPDATE orders SET filled_quantity = filled_quantity + ?1,
                     status = CASE WHEN filled_quantity + ?1 >= quantity THEN 'filled' ELSE 'partial' END,
                     updated_at = unixepoch() WHERE id = ?2",
                    params![quantity, buy_id],
                )?;

                tx.execute(
                    "UPDATE orders SET filled_quantity = filled_quantity + ?1,
                     status = CASE WHEN filled_quantity + ?1 >= quantity THEN 'filled' ELSE 'partial' END,
                     updated_at = unixepoch() WHERE id = ?2",
                    params![quantity, sell_id],
                )?;
            }
        }
        Ok(())
    }

    fn update_balance_in_tx(user_ids: &[i64], tx: &rusqlite::Transaction) -> Result<()> {
        let mut rng = rand::thread_rng();
        let user_id = user_ids[rng.gen_range(0..user_ids.len())];
        let symbol = SYMBOLS[rng.gen_range(0..SYMBOLS.len())];
        let amount_change: i64 = rng.gen_range(-1000..1000);

        tx.execute(
            "UPDATE balances SET amount = MAX(0, amount + ?1), updated_at = unixepoch()
             WHERE user_id = ?2 AND symbol = ?3",
            params![amount_change, user_id, symbol],
        )?;
        Ok(())
    }
}

struct DbStats {
    total_orders: i64,
    active_orders: i64,
    total_trades: i64,
}
