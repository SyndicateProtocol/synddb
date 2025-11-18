//! Example demonstrating snapshot creation and restoration

use anyhow::Result;
use rusqlite::Connection;
use synddb_client::SyndDB;

fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    println!("=== SyndDB Snapshot Example ===\n");

    // Create source database
    let source_conn = Box::leak(Box::new(Connection::open("source.db")?));

    // Create schema and populate with data
    source_conn.execute(
        "CREATE TABLE IF NOT EXISTS users (
            id INTEGER PRIMARY KEY,
            name TEXT,
            balance INTEGER
        )",
        [],
    )?;

    println!("✓ Source database created");

    // Insert some data
    for i in 1..=5 {
        source_conn.execute(
            "INSERT OR REPLACE INTO users (id, name, balance) VALUES (?1, ?2, ?3)",
            rusqlite::params![i, format!("User {}", i), 100 * i],
        )?;
    }

    println!("✓ Inserted 5 users into source database");

    // Attach SyndDB
    let synddb = SyndDB::attach(source_conn, "http://localhost:8433")?;
    println!("✓ SyndDB attached to source database\n");

    // Wait a moment for initial flush
    std::thread::sleep(std::time::Duration::from_secs(1));

    // Create snapshot
    println!("Creating snapshot...");
    let snapshot = synddb.snapshot()?;

    println!("✓ Snapshot created:");
    println!("  - Size: {} bytes", snapshot.data.len());
    println!("  - Sequence: {}", snapshot.sequence);
    println!("  - Timestamp: {:?}\n", snapshot.timestamp);

    // Restore snapshot to a new database file to verify portability
    println!("Restoring snapshot to new database...");
    std::fs::write("restored.db", &snapshot.data)?;

    // Verify restored database
    let restored_conn = Connection::open("restored.db")?;
    let count: i64 = restored_conn.query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0))?;

    println!("✓ Snapshot restored successfully");
    println!("  - Restored database has {} users", count);

    // Verify data integrity
    {
        let mut stmt = restored_conn.prepare("SELECT id, name, balance FROM users ORDER BY id")?;
        let users: Vec<(i64, String, i64)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
            .collect::<Result<Vec<_>, _>>()?;

        println!("\n  Restored users:");
        for (id, name, balance) in users {
            println!("    - {} (id={}, balance={})", name, id, balance);
        }
    }

    println!("\n✓ Cross-platform snapshot successfully created and restored!");
    println!("  The snapshot.data bytes can be sent to any machine/architecture");

    // Cleanup
    drop(restored_conn);
    std::fs::remove_file("source.db").ok();
    std::fs::remove_file("restored.db").ok();

    Ok(())
}
