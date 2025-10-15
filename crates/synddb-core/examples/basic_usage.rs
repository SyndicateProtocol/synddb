//! Basic usage example for SyndDB Core
//!
//! Run with: cargo run --example basic_usage

use std::sync::Arc;
use synddb_core::{
    database::{SqliteDatabase, SyndDatabase},
    types::SqlValue,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing for logs
    tracing_subscriber::fmt::init();

    println!("=== SyndDB Core Basic Usage Example ===\n");

    // Create a database
    let db = Arc::new(SqliteDatabase::new("example.db", 4)?);
    println!("✓ Created database at example.db\n");

    // Create a table
    db.execute(
        "CREATE TABLE IF NOT EXISTS users (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            email TEXT NOT NULL,
            balance INTEGER DEFAULT 0,
            created_at INTEGER NOT NULL
        )",
        vec![],
    )
    .await?;
    println!("✓ Created users table\n");

    // Insert some data
    println!("Inserting users...");
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs() as i64;

    for (name, email, balance) in [
        ("Alice", "alice@example.com", 1000),
        ("Bob", "bob@example.com", 2500),
        ("Charlie", "charlie@example.com", 500),
    ] {
        let result = db
            .execute(
                "INSERT INTO users (name, email, balance, created_at) VALUES (?1, ?2, ?3, ?4)",
                vec![
                    SqlValue::Text(name.to_string()),
                    SqlValue::Text(email.to_string()),
                    SqlValue::Integer(balance),
                    SqlValue::Integer(timestamp),
                ],
            )
            .await?;

        println!(
            "  ✓ Inserted {} (id: {:?})",
            name,
            result.last_insert_rowid.unwrap()
        );
    }
    println!();

    // Query all users
    println!("Querying all users...");
    let results = db
        .query("SELECT * FROM users ORDER BY balance DESC", vec![])
        .await?;

    println!("  Found {} users:", results.row_count);
    for row in &results.rows {
        if let [SqlValue::Integer(id), SqlValue::Text(name), SqlValue::Text(email), SqlValue::Integer(balance), SqlValue::Integer(_created)] =
            &row[..]
        {
            println!(
                "    - {} (id: {}, email: {}, balance: {})",
                name, id, email, balance
            );
        }
    }
    println!();

    // Update a balance
    println!("Updating Bob's balance...");
    db.execute(
        "UPDATE users SET balance = balance + ?1 WHERE name = ?2",
        vec![SqlValue::Integer(500), SqlValue::Text("Bob".to_string())],
    )
    .await?;
    println!("  ✓ Added 500 to Bob's balance\n");

    // Query specific user
    println!("Querying Bob's updated balance...");
    let result = db
        .query(
            "SELECT name, balance FROM users WHERE name = ?1",
            vec![SqlValue::Text("Bob".to_string())],
        )
        .await?;

    if let Some(row) = result.rows.first() {
        if let [SqlValue::Text(name), SqlValue::Integer(balance)] = &row[..] {
            println!("  {} now has balance: {}\n", name, balance);
        }
    }

    // Batch insert with transaction
    println!("Batch inserting 3 more users...");
    let operations = vec![
        synddb_core::types::SqlOperation {
            sql: "INSERT INTO users (name, email, balance, created_at) VALUES (?1, ?2, ?3, ?4)"
                .to_string(),
            params: vec![
                SqlValue::Text("Dave".to_string()),
                SqlValue::Text("dave@example.com".to_string()),
                SqlValue::Integer(750),
                SqlValue::Integer(timestamp),
            ],
        },
        synddb_core::types::SqlOperation {
            sql: "INSERT INTO users (name, email, balance, created_at) VALUES (?1, ?2, ?3, ?4)"
                .to_string(),
            params: vec![
                SqlValue::Text("Eve".to_string()),
                SqlValue::Text("eve@example.com".to_string()),
                SqlValue::Integer(1200),
                SqlValue::Integer(timestamp),
            ],
        },
        synddb_core::types::SqlOperation {
            sql: "INSERT INTO users (name, email, balance, created_at) VALUES (?1, ?2, ?3, ?4)"
                .to_string(),
            params: vec![
                SqlValue::Text("Frank".to_string()),
                SqlValue::Text("frank@example.com".to_string()),
                SqlValue::Integer(300),
                SqlValue::Integer(timestamp),
            ],
        },
    ];

    let batch_result = db.execute_batch(operations).await?;
    println!(
        "  ✓ Batch inserted 3 users in {:?}\n",
        batch_result.duration
    );

    // Final count
    let count_result = db
        .query("SELECT COUNT(*) as count FROM users", vec![])
        .await?;

    if let Some(row) = count_result.rows.first() {
        if let [SqlValue::Integer(count)] = &row[..] {
            println!("Total users in database: {}\n", count);
        }
    }

    println!("=== SQLite Optimization ===");
    println!("  Using SQLite's built-in prepared statement optimization");
    println!("  Performance metrics available via tracing/logging");

    println!("\n✓ Example complete!");
    println!("\nDatabase file created at: example.db");
    println!("You can inspect it with: sqlite3 example.db");

    Ok(())
}
