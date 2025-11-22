use anyhow::Result;
use clap::{Parser, Subcommand};
use rusqlite::Connection;
use std::path::PathBuf;
use tracing::{info, warn};

use load_patterns::{LoadConfig, LoadPattern};
use orderbook::OrderbookSimulator;
use synddb_benchmark::{load_patterns, orderbook, schema};

#[derive(Parser)]
#[command(name = "orderbook-bench")]
#[command(about = "Orderbook benchmark tool for SyndDB sequencer development", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize the database with orderbook schema
    Init {
        /// Path to the `SQLite` database
        #[arg(short, long, default_value = "orderbook.db")]
        db: PathBuf,
    },
    /// Run the orderbook simulation
    Run {
        /// Path to the `SQLite` database
        #[arg(short, long, default_value = "orderbook.db")]
        db: PathBuf,

        /// Clean all existing data before starting (default: resume with existing data)
        #[arg(long, default_value = "false")]
        clean: bool,

        /// Load pattern: continuous or burst
        #[arg(short, long, default_value = "continuous")]
        pattern: String,

        /// Operations per second (for continuous mode, 0 = auto-find max throughput)
        #[arg(short, long, default_value = "100")]
        rate: u64,

        /// Duration in seconds (0 = run forever)
        #[arg(short = 't', long, default_value = "0")]
        duration: u64,

        /// Burst size (for burst mode)
        #[arg(short, long, default_value = "1000")]
        burst_size: usize,

        /// Pause between bursts in seconds (for burst mode)
        #[arg(short = 'i', long, default_value = "5")]
        burst_interval: u64,

        /// Batch size for transaction grouping (higher = faster writes)
        #[arg(long, default_value = "100")]
        batch_size: usize,

        /// Simple mode: only insert orders (no queries, much faster)
        #[arg(long, default_value = "false")]
        simple: bool,
    },
    /// Show statistics about the database
    Stats {
        /// Path to the `SQLite` database
        #[arg(short, long, default_value = "orderbook.db")]
        db: PathBuf,
    },
    /// Clear all data from the database
    Clear {
        /// Path to the `SQLite` database
        #[arg(short, long, default_value = "orderbook.db")]
        db: PathBuf,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_target(false)
        .with_thread_ids(false)
        .with_level(true)
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Init { db } => {
            info!("Initializing database at {:?}", db);
            let conn = Connection::open(&db)?;
            schema::initialize_schema(&conn)?;
            info!("Database initialized successfully");
            info!("Schema includes: users, orders, trades, balances tables");
        }
        Commands::Run {
            db,
            clean,
            pattern,
            rate,
            duration,
            burst_size,
            burst_interval,
            batch_size,
            simple,
        } => {
            info!("Starting orderbook simulation at {:?}", db);

            let load_pattern = if rate == 0 {
                info!("Rate set to 0, enabling max throughput discovery mode");
                LoadPattern::MaxThroughput
            } else {
                match pattern.as_str() {
                    "continuous" => LoadPattern::Continuous {
                        ops_per_second: rate,
                    },
                    "burst" => LoadPattern::Burst {
                        burst_size,
                        pause_seconds: burst_interval,
                    },
                    _ => {
                        warn!("Unknown pattern '{}', defaulting to continuous", pattern);
                        LoadPattern::Continuous {
                            ops_per_second: rate,
                        }
                    }
                }
            };

            let config = LoadConfig {
                pattern: load_pattern,
                duration_seconds: if duration == 0 { None } else { Some(duration) },
                batch_size,
                simple_mode: simple,
            };

            let conn = Connection::open(&db)?;

            // Ensure schema exists
            schema::initialize_schema(&conn)?;

            // Clean data if requested
            if clean {
                info!("Cleaning existing data...");
                schema::clear_data(&conn)?;
                info!("Data cleaned successfully");
            } else {
                // Check if there's existing data
                let user_count: i64 =
                    conn.query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0))?;
                if user_count > 0 {
                    info!("Resuming with existing data ({} users found)", user_count);
                }
            }

            let mut simulator = OrderbookSimulator::new(conn);
            simulator.run(config).await?;
        }
        Commands::Stats { db } => {
            let conn = Connection::open(&db)?;
            show_stats(&conn)?;
        }
        Commands::Clear { db } => {
            info!("Clearing all data from {:?}", db);
            let conn = Connection::open(&db)?;
            schema::clear_data(&conn)?;
            info!("All data cleared successfully");
        }
    }

    Ok(())
}

fn show_stats(conn: &Connection) -> Result<()> {
    let user_count: i64 = conn.query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0))?;
    let order_count: i64 = conn.query_row("SELECT COUNT(*) FROM orders", [], |row| row.get(0))?;
    let trade_count: i64 = conn.query_row("SELECT COUNT(*) FROM trades", [], |row| row.get(0))?;

    let active_orders: i64 = conn.query_row(
        "SELECT COUNT(*) FROM orders WHERE status = 'active'",
        [],
        |row| row.get(0),
    )?;
    let filled_orders: i64 = conn.query_row(
        "SELECT COUNT(*) FROM orders WHERE status = 'filled'",
        [],
        |row| row.get(0),
    )?;
    let cancelled_orders: i64 = conn.query_row(
        "SELECT COUNT(*) FROM orders WHERE status = 'cancelled'",
        [],
        |row| row.get(0),
    )?;

    info!("=== Orderbook Statistics ===");
    info!("Users:           {}", user_count);
    info!("Orders:          {} total", order_count);
    info!("  - Active:      {}", active_orders);
    info!("  - Filled:      {}", filled_orders);
    info!("  - Cancelled:   {}", cancelled_orders);
    info!("Trades:          {}", trade_count);

    Ok(())
}
