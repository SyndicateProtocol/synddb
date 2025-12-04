use anyhow::Result;
use clap::{Parser, Subcommand};
use rusqlite::Connection;
use std::path::PathBuf;
use std::time::Duration;
use tracing::{info, warn};

use load_patterns::{LoadConfig, LoadPattern};
use orderbook::OrderbookSimulator;
use synddb_benchmark::{load_patterns, orderbook, schema};
use synddb_client::SyndDB;

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

        /// Sequencer URL for sending changesets (enables `SyndDB` integration)
        ///
        /// When set, the benchmark will capture `SQLite` changesets and send them
        /// to the sequencer for ordering and signing.
        #[arg(long, env = "SEQUENCER_URL")]
        sequencer_url: Option<String>,

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
            sequencer_url,
            clean,
            pattern,
            rate,
            duration,
            burst_size,
            burst_interval,
            batch_size,
            simple,
        } => {
            info!("===========================================");
            info!("  SyndDB Orderbook Benchmark");
            info!("===========================================");
            info!(database = ?db, "Database path");

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

            info!(
                pattern = %pattern,
                rate = rate,
                duration = duration,
                batch_size = batch_size,
                simple_mode = simple,
                "Configuration"
            );

            // Create connection with 'static lifetime for SyndDB integration
            let conn: &'static Connection = Box::leak(Box::new(Connection::open(&db)?));

            // Ensure schema exists
            info!("Initializing database schema...");
            schema::initialize_schema(conn)?;
            info!("Schema ready: users, orders, trades, balances tables");

            // Clean data if requested
            if clean {
                info!("Cleaning existing data...");
                schema::clear_data(conn)?;
                info!("Data cleaned successfully");
            } else {
                // Check if there's existing data
                let user_count: i64 =
                    conn.query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0))?;
                if user_count > 0 {
                    info!(users = user_count, "Resuming with existing data");
                }
            }

            // Attach SyndDB if sequencer URL is provided
            info!("-------------------------------------------");
            let synddb = if let Some(ref url) = sequencer_url {
                info!(url = %url, "SyndDB Integration: ENABLED");
                info!("Changesets will be captured and sent to sequencer");
                match SyndDB::attach(conn, url) {
                    Ok(synddb) => {
                        info!("SyndDB client attached successfully");
                        Some(synddb)
                    }
                    Err(e) => {
                        warn!(error = %e, "Failed to attach SyndDB client, continuing without it");
                        None
                    }
                }
            } else {
                info!("SyndDB Integration: DISABLED");
                info!("Changesets will NOT be sent to sequencer");
                None
            };
            info!("-------------------------------------------");

            info!("Starting simulation...");
            let mut simulator = OrderbookSimulator::new(conn);
            if let Some(ref sdb) = synddb {
                simulator = simulator.with_synddb(sdb);
            }
            simulator.run(config).await?;

            // Give SyndDB time to flush any pending changesets before exit
            if synddb.is_some() {
                info!("Flushing pending changesets to sequencer...");
                tokio::time::sleep(Duration::from_secs(3)).await;
                info!("Flush complete");
            }

            info!("===========================================");
            info!("  Benchmark Complete");
            info!("===========================================");
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
