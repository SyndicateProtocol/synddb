//! Example Rust application using synddb-client

use anyhow::Result;
use rusqlite::Connection;
use synddb_client::SyndDB;

fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    println!("=== SyndDB Client Example ===\n");

    // Open database
    // Note: We use Box::leak to get 'static lifetime for the connection
    // In a real application, you'd manage the connection lifetime appropriately
    let conn = Box::leak(Box::new(Connection::open("example.db")?));

    // Create schema
    conn.execute(
        "CREATE TABLE IF NOT EXISTS trades (
            id INTEGER PRIMARY KEY,
            price INTEGER,
            quantity INTEGER,
            timestamp INTEGER
        )",
        [],
    )?;

    println!("✓ Database opened and schema created");

    // INTEGRATION POINT: Single line to enable SyndDB
    let _synddb = SyndDB::attach(conn, "http://localhost:8433")?;
    println!("✓ SyndDB client attached to connection\n");

    // Application code - completely unchanged from here
    println!("Executing trades...");

    for i in 1..=10 {
        conn.execute(
            "INSERT INTO trades (id, price, quantity, timestamp) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![
                i,
                100 + i,
                10,
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs()
            ],
        )?;
        println!("  Trade {} inserted", i);

        // Simulate some delay
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    println!("\n✓ All trades executed");
    println!("✓ Changesets are automatically captured and sent to sequencer");
    println!("  (flushed every 1 second by default)");

    // Wait a moment to let automatic flush happen
    println!("\nWaiting 2 seconds for automatic flush...");
    std::thread::sleep(std::time::Duration::from_secs(2));

    // You can also manually flush for critical transactions:
    // synddb.flush()?;

    Ok(())
}
