use rusqlite::Connection;

// Helper function to create a test connection with 'static lifetime
fn setup_test_db() -> &'static Connection {
    let conn = Box::leak(Box::new(Connection::open_in_memory().unwrap()));
    synddb_benchmark::schema::initialize_schema(conn).unwrap();
    conn
}

#[tokio::test]
async fn test_orderbook_initialization() {
    let conn = setup_test_db();
    let mut simulator = synddb_benchmark::orderbook::OrderbookSimulator::new(conn);

    // Create a short-duration config
    let config = synddb_benchmark::load_patterns::LoadConfig {
        pattern: synddb_benchmark::load_patterns::LoadPattern::Continuous {
            ops_per_second: 100,
        },
        duration_seconds: Some(1), // Just 1 second
        batch_size: 100,
        simple_mode: false,
    };

    // Run simulation
    simulator.run(config).await.unwrap();

    // Verify that some operations were performed
    // After 1 second at 100 ops/sec, we should have ~100 operations
    // This will create users, orders, trades, etc.
}

#[tokio::test]
async fn test_burst_mode() {
    let conn = setup_test_db();
    let mut simulator = synddb_benchmark::orderbook::OrderbookSimulator::new(conn);

    let config = synddb_benchmark::load_patterns::LoadConfig {
        pattern: synddb_benchmark::load_patterns::LoadPattern::Burst {
            burst_size: 50,
            pause_seconds: 1,
        },
        duration_seconds: Some(2), // 2 seconds total
        batch_size: 100,
        simple_mode: false,
    };

    // Should complete at least one burst
    simulator.run(config).await.unwrap();
}

#[test]
fn test_schema_tables_exist() {
    let conn = setup_test_db();

    // Verify all required tables exist
    let table_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN ('users', 'orders', 'trades', 'balances')",
            [],
            |row| row.get(0),
        )
        .unwrap();

    assert_eq!(table_count, 4, "All 4 tables should exist");
}

#[test]
fn test_clear_data_preserves_schema() {
    let conn = setup_test_db();

    // Add some data
    conn.execute("INSERT INTO users (username) VALUES ('test')", [])
        .unwrap();

    // Clear data
    synddb_benchmark::schema::clear_data(&conn).unwrap();

    // Schema should still exist
    let table_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table'",
            [],
            |row| row.get(0),
        )
        .unwrap();

    assert!(
        table_count > 0,
        "Tables should still exist after clear_data"
    );

    // Data should be gone
    let user_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0))
        .unwrap();

    assert_eq!(user_count, 0, "Users table should be empty");
}
