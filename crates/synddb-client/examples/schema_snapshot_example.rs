//! Example demonstrating immediate snapshots on schema changes (DDL operations)

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

    // Create database
    let conn = Box::leak(Box::new(Connection::open("schema_test.db")?));

    // Initial schema
    conn.execute(
        "CREATE TABLE users (
            id INTEGER PRIMARY KEY,
            name TEXT
        )",
        [],
    )?;

    println!("✓ Initial table created\n");

    // Configure with high snapshot_interval to show schema changes override it
    let config = Config {
        sequencer_url: "http://localhost:8433".to_string(),
        publish_interval: Duration::from_millis(300),
        snapshot_interval: 100, // High interval - schema changes should trigger snapshots anyway
        ..Default::default()
    };

    println!("Config:");
    println!("  - Publish interval: {:?}", config.publish_interval);
    println!(
        "  - Snapshot interval: {} changesets (high on purpose)\n",
        config.snapshot_interval
    );

    // Attach SyndDB
    let _synddb = SyndDB::attach_with_config(conn, config)?;
    println!("✓ SyndDB attached\n");

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

    // Now perform schema change - should trigger immediate snapshot
    println!("Performing schema change (ALTER TABLE to add column)...");
    conn.execute("ALTER TABLE users ADD COLUMN email TEXT", [])?;
    println!("  ✓ Added 'email' column\n");

    // Wait for snapshot to be captured
    std::thread::sleep(Duration::from_millis(500));

    println!("Adding another column...");
    conn.execute("ALTER TABLE users ADD COLUMN age INTEGER", [])?;
    println!("  ✓ Added 'age' column\n");

    // Wait for snapshot
    std::thread::sleep(Duration::from_millis(500));

    println!("Creating a new table...");
    conn.execute(
        "CREATE TABLE products (
            id INTEGER PRIMARY KEY,
            name TEXT,
            price INTEGER
        )",
        [],
    )?;
    println!("  ✓ Created 'products' table\n");

    // Wait for final snapshot
    std::thread::sleep(Duration::from_secs(1));

    println!("\n✓ Test complete!");
    println!(
        "  Check logs above for 'Schema change detected' and 'Schema change snapshot' messages"
    );
    println!("  Expected: 3 schema change snapshots (ALTER, ALTER, CREATE TABLE)");

    // Cleanup
    std::fs::remove_file("schema_test.db").ok();

    Ok(())
}
