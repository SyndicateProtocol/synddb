//! Example: SyndDB Client with Chain Monitor
//!
//! This example demonstrates how to use the SyndDB client with blockchain chain monitoring.
//! It shows the complete integration:
//! 1. SQLite database captures local changes
//! 2. Chain monitor listens for blockchain deposit events
//! 3. Deposits are inserted into the local database
//! 4. All changes are automatically published to the sequencer
//!
//! Run with:
//! ```bash
//! WS_URL=wss://base-mainnet.g.alchemy.com/v2/YOUR_KEY \
//! CONTRACT_ADDRESS=0x1234567890123456789012345678901234567890 \
//! START_BLOCK=10000000 \
//! cargo run --example chain_monitor_example --features chain-monitor
//! ```

#[cfg(feature = "chain-monitor")]
use rusqlite::Connection;
#[cfg(feature = "chain-monitor")]
use synddb_client::{Config, SyndDB};
#[cfg(feature = "chain-monitor")]
use std::thread;
#[cfg(feature = "chain-monitor")]
use std::time::Duration;

#[cfg(feature = "chain-monitor")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .init();

    println!("SyndDB Client with Chain Monitor Example");
    println!("=========================================\n");

    // Create SQLite database
    let conn = Box::leak(Box::new(Connection::open("example.db")?));

    // Create application tables
    conn.execute(
        "CREATE TABLE IF NOT EXISTS trades (
            id INTEGER PRIMARY KEY,
            amount INTEGER NOT NULL,
            timestamp INTEGER NOT NULL
        )",
        [],
    )?;
    println!("✓ Created application tables");

    // Configure SyndDB with chain monitor
    let ws_url = std::env::var("WS_URL")
        .unwrap_or_else(|_| "wss://base-mainnet.g.alchemy.com/v2/YOUR_KEY".to_string());
    let contract_address = std::env::var("CONTRACT_ADDRESS")
        .unwrap_or_else(|_| "0x0000000000000000000000000000000000000000".to_string());
    let start_block: u64 = std::env::var("START_BLOCK")
        .unwrap_or_else(|_| "0".to_string())
        .parse()?;

    let config = Config {
        sequencer_url: "http://localhost:8433".to_string(),
        publish_interval: Duration::from_secs(5),
        chain_monitor: Some(synddb_client::config::ChainMonitorConfig {
            ws_urls: vec![ws_url.clone()],
            contract_address: contract_address.clone(),
            start_block,
            event_signature: None, // Monitor all events
            event_store_path: "./chain_events.db".to_string(),
            deposit_table: "deposits".to_string(),
        }),
        ..Default::default()
    };

    println!("✓ Configuration:");
    println!("  - Sequencer: {}", config.sequencer_url);
    println!("  - Chain WS: {}", ws_url);
    println!("  - Contract: {}", contract_address);
    println!("  - Start block: {}\n", start_block);

    // Attach SyndDB client
    let synddb = SyndDB::attach_with_config(conn, config)?;
    println!("✓ SyndDB client attached");
    println!("✓ Chain monitor started\n");

    // Simulate application activity
    println!("Simulating application activity...");
    let mut trade_id = 1;

    for i in 0..10 {
        // Insert some trades
        conn.execute(
            "INSERT INTO trades (id, amount, timestamp) VALUES (?1, ?2, ?3)",
            rusqlite::params![
                trade_id,
                1000 + (i * 100),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_secs()
            ],
        )?;
        println!("  → Inserted trade #{}", trade_id);
        trade_id += 1;

        // Process any pending deposits from blockchain
        match synddb.process_deposits() {
            Ok(count) if count > 0 => {
                println!("  ✓ Processed {} deposits from blockchain", count);
            }
            Ok(_) => {
                // No deposits to process
            }
            Err(e) => {
                eprintln!("  ✗ Error processing deposits: {}", e);
            }
        }

        // Wait a bit
        thread::sleep(Duration::from_secs(2));
    }

    println!("\n✓ Application activity complete");
    println!("  - {} trades created", trade_id - 1);

    // Final deposit processing
    match synddb.process_deposits() {
        Ok(count) if count > 0 => {
            println!("  - {} deposits processed from blockchain", count);
        }
        _ => {}
    }

    // Check deposits table
    let deposit_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM deposits",
        [],
        |row| row.get(0),
    )?;
    println!("  - {} total deposits in database", deposit_count);

    // Show some stats
    if let Some(stats) = synddb.recovery_stats()? {
        println!("\nRecovery Stats:");
        println!("  - Failed changesets: {}", stats.failed_changesets);
        println!("  - Failed snapshots: {}", stats.failed_snapshots);
    }

    println!("\n📊 Summary:");
    println!("  1. Application created {} trades in local SQLite", trade_id - 1);
    println!("  2. Chain monitor detected {} deposits from blockchain", deposit_count);
    println!("  3. All changes are being published to sequencer every 5s");
    println!("  4. Sequencer will replicate everything to other replicas\n");

    println!("Press Ctrl+C to stop...");

    // Keep running to continue monitoring
    loop {
        thread::sleep(Duration::from_secs(5));

        // Periodically process deposits
        if let Ok(count) = synddb.process_deposits() {
            if count > 0 {
                println!("✓ Processed {} new deposits", count);
            }
        }
    }
}

#[cfg(not(feature = "chain-monitor"))]
fn main() {
    eprintln!("This example requires the 'chain-monitor' feature.");
    eprintln!("Run with: cargo run --example chain_monitor_example --features chain-monitor");
    std::process::exit(1);
}
