//! Example demonstrating automatic snapshot creation based on changeset count
//!
//! **Complexity:** Intermediate
//! **Features:** Automatic snapshots, custom configuration
//! **Prerequisites:** Sequencer running on localhost:8433
//! **Run:** `cargo run --example auto_snapshot_example`

use anyhow::Result;
use rusqlite::Connection;
use std::time::Duration;
use synddb_client::{Config, SyndDB};

fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    println!("=== SyndDB Automatic Snapshot Example ===\n");

    // Create database
    let conn = Box::leak(Box::new(Connection::open("auto_snapshot.db")?));

    // Create schema
    conn.execute(
        "CREATE TABLE IF NOT EXISTS events (
            id INTEGER PRIMARY KEY,
            name TEXT,
            value INTEGER
        )",
        [],
    )?;

    println!("✓ Database created\n");

    // Configure SyndDB with automatic snapshots every 5 changesets
    let config = Config {
        sequencer_url: "http://localhost:8433".parse().unwrap(),
        flush_interval: Duration::from_millis(500), // Publish every 500ms
        snapshot_interval: 5,                         // Snapshot every 5 changesets
        ..Default::default()
    };

    println!("Config:");
    println!("  - Flush interval: {:?}", config.flush_interval);
    println!(
        "  - Snapshot interval: {} changesets\n",
        config.snapshot_interval
    );

    // Attach SyndDB with custom config
    let _synddb = SyndDB::attach_with_config(conn, config)?;

    println!("✓ SyndDB attached with automatic snapshots enabled\n");

    // Insert changesets and observe automatic snapshots
    println!("Inserting events (watch for automatic snapshots)...\n");

    for i in 1..=15 {
        conn.execute(
            "INSERT INTO events (id, name, value) VALUES (?1, ?2, ?3)",
            rusqlite::params![i, format!("Event {}", i), i * 10],
        )?;
        println!("  Event {} inserted", i);

        // Wait a moment for publish
        std::thread::sleep(Duration::from_millis(600));
    }

    println!("\n✓ All events inserted");
    println!("  Expected snapshots: 3 (at changesets 5, 10, and 15)");
    println!("  Check logs above for 'automatic snapshot' messages\n");

    // Wait to let final snapshot complete
    std::thread::sleep(Duration::from_secs(2));

    // Cleanup
    std::fs::remove_file("auto_snapshot.db").ok();

    Ok(())
}
