//! Example demonstrating immediate snapshots on schema changes (DDL operations)
//!
//! This example shows that DDL operations automatically trigger snapshots:
//! - `execute_ddl()`: Executes DDL and automatically publishes snapshot
//! - Direct connection DDL still detected via SQLite hooks
//!
//! **Complexity:** Advanced
//! **Features:** Schema change detection, DDL-triggered snapshots
//! **Prerequisites:** Sequencer running on localhost:8433
//! **Run:** `cargo run --example schema_snapshot_example`

use anyhow::Result;
use rusqlite::Connection;
use std::time::Duration;
use synddb_client::{Config, SyndDB};

fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    println!("=== SyndDB Schema Change Snapshot Example ===\n");
    println!("This demonstrates that DDL operations (CREATE TABLE, ALTER TABLE, etc.)");
    println!("trigger immediate snapshots, regardless of snapshot_interval\n");

    // Create database and attach SyndDB FIRST
    let conn = Box::leak(Box::new(Connection::open("schema_test.db")?));

    // Configure with high snapshot_interval to show DDL snapshots are independent
    let config = Config {
        sequencer_url: "http://localhost:8433".parse().unwrap(),
        flush_interval: Duration::from_millis(300),
        snapshot_interval: 100, // High interval - DDL triggers snapshots anyway
        ..Default::default()
    };

    println!("Config:");
    println!("  - Flush interval: {:?}", config.flush_interval);
    println!(
        "  - Snapshot interval: {} changesets (high on purpose)",
        config.snapshot_interval
    );
    println!("  - Auto-snapshot on DDL: enabled (default)\n");

    // Attach SyndDB
    let synddb = SyndDB::attach_with_config(conn, config)?;
    println!("✓ SyndDB attached\n");

    // Create initial schema using execute_ddl()
    println!("Creating initial table (will trigger automatic snapshot)...");
    synddb.execute_ddl(
        "CREATE TABLE users (
            id INTEGER PRIMARY KEY,
            name TEXT
        )",
    )?;
    println!("✓ Initial table created (snapshot published)\n");

    // Insert a few rows (not enough to trigger sequence-based snapshot)
    println!("Inserting 3 users (not enough to trigger sequence snapshot)...");
    for i in 1..=3 {
        conn.execute(
            "INSERT INTO users (id, name) VALUES (?1, ?2)",
            rusqlite::params![i, format!("User {}", i)],
        )?;
        std::thread::sleep(Duration::from_millis(350));
    }
    println!("  ✓ 3 users inserted\n");

    // Wait a moment
    std::thread::sleep(Duration::from_millis(500));

    // Perform schema changes using execute_ddl() - each triggers automatic snapshot
    println!("Performing schema change (ALTER TABLE to add column)...");
    synddb.execute_ddl("ALTER TABLE users ADD COLUMN email TEXT")?;
    println!("  ✓ Added 'email' column (snapshot published)\n");

    // Wait a moment
    std::thread::sleep(Duration::from_millis(500));

    println!("Adding another column...");
    synddb.execute_ddl("ALTER TABLE users ADD COLUMN age INTEGER")?;
    println!("  ✓ Added 'age' column (snapshot published)\n");

    // Wait a moment
    std::thread::sleep(Duration::from_millis(500));

    println!("Creating a new table...");
    synddb.execute_ddl(
        "CREATE TABLE products (
            id INTEGER PRIMARY KEY,
            name TEXT,
            price INTEGER
        )",
    )?;
    println!("  ✓ Created 'products' table (snapshot published)\n");

    // Wait for final operations to complete
    std::thread::sleep(Duration::from_secs(1));

    println!("\n✓ Test complete!");
    println!("  Each DDL operation automatically triggered a snapshot.");
    println!("  Expected: 4 snapshots (initial CREATE + 2 ALTER + 1 CREATE)");

    // Cleanup
    std::fs::remove_file("schema_test.db").ok();

    Ok(())
}
