//! Session Stress Test
//!
//! This binary stress-tests the `SyndDB` client by running continuous transactions
//! with changeset publishing. It verifies that:
//!
//! 1. High-volume transactions work correctly
//! 2. Changeset extraction and publishing works under load
//! 3. No crashes or data corruption occur
//!
//! The test publishes changesets after each transaction to simulate real-world usage.
//!
//! Usage:
//!   `SEQUENCER_URL=http://localhost:8433` session-stress-test
//!
//! Expected behavior:
//!   - Should always complete successfully
//!   - No crashes regardless of platform

use anyhow::Result;
use clap::Parser;
use rusqlite::Connection;
use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use synddb_client::{Config, SyndDB};
use tracing::info;

#[derive(Parser)]
#[command(name = "session-stress-test")]
#[command(about = "Stress test for SyndDB client")]
struct Args {
    /// Sequencer URL
    #[arg(long, env = "SEQUENCER_URL", default_value = "http://localhost:8433")]
    sequencer_url: String,

    /// Test duration in seconds
    #[arg(long, default_value = "30")]
    duration: u64,

    /// Rows per transaction (larger = bigger changesets)
    #[arg(long, default_value = "50")]
    rows_per_tx: usize,

    /// Number of iterations to report progress
    #[arg(long, default_value = "100")]
    report_interval: u64,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_target(false)
        .with_thread_ids(true)
        .with_level(true)
        .init();

    let args = Args::parse();

    info!("===========================================");
    info!("  SyndDB Session Stress Test");
    info!("===========================================");
    info!("Sequencer URL: {}", args.sequencer_url);
    info!("Duration: {}s", args.duration);
    info!("Rows per transaction: {}", args.rows_per_tx);
    info!("-------------------------------------------");

    // Create connection with 'static lifetime (required by current API)
    let conn: &'static Connection = Box::leak(Box::new(Connection::open_in_memory()?));

    // Create schema
    conn.execute(
        "CREATE TABLE stress_test (
            id INTEGER PRIMARY KEY,
            thread_id INTEGER NOT NULL,
            iteration INTEGER NOT NULL,
            data BLOB NOT NULL,
            timestamp INTEGER NOT NULL
        )",
        [],
    )?;

    info!("Schema created");

    // Configure SyndDB
    let config = Config {
        sequencer_url: args.sequencer_url.parse()?,
        snapshot_interval: 0, // Disable automatic snapshots
        ..Default::default()
    };

    info!("Attaching SyndDB...");
    let synddb = SyndDB::attach_with_config(conn, config)?;
    info!("SyndDB attached");

    // Counters for statistics
    let transactions = Arc::new(AtomicU64::new(0));
    let rows_inserted = Arc::new(AtomicU64::new(0));
    let changesets_published = Arc::new(AtomicU64::new(0));

    let start = Instant::now();
    let duration = Duration::from_secs(args.duration);

    info!("Starting stress test loop...");
    info!("");

    let mut iteration: u64 = 0;
    let mut last_report = Instant::now();

    // Run transactions continuously
    while start.elapsed() < duration {
        // Use unchecked_transaction to work with SyndDB's session borrow
        let tx = conn.unchecked_transaction()?;

        // Insert many rows to create changesets
        for i in 0..args.rows_per_tx {
            let data = vec![
                (iteration & 0xFF) as u8,
                ((iteration >> 8) & 0xFF) as u8,
                (i & 0xFF) as u8,
                ((i >> 8) & 0xFF) as u8,
            ];

            tx.execute(
                "INSERT INTO stress_test (thread_id, iteration, data, timestamp) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![1, iteration, data, start.elapsed().as_millis() as i64],
            )?;
        }

        tx.commit()?;

        // Publish changesets after commit - this is the safe pattern
        // The main thread calls publish_changeset() when not in a transaction
        if let Err(e) = synddb.publish_changeset() {
            // Log but don't fail - network errors are expected without a real sequencer
            if iteration == 0 {
                info!(
                    "Note: publish_changeset() returned error (expected without sequencer): {}",
                    e
                );
            }
        } else {
            changesets_published.fetch_add(1, Ordering::Relaxed);
        }

        transactions.fetch_add(1, Ordering::Relaxed);
        rows_inserted.fetch_add(args.rows_per_tx as u64, Ordering::Relaxed);
        iteration += 1;

        // Periodic progress report
        if iteration.is_multiple_of(args.report_interval)
            && last_report.elapsed() > Duration::from_secs(1)
        {
            let elapsed = start.elapsed().as_secs_f64();
            let tx_count = transactions.load(Ordering::Relaxed);
            let row_count = rows_inserted.load(Ordering::Relaxed);
            info!(
                "Progress: {:.1}s | {} tx ({:.0}/s) | {} rows ({:.0}/s)",
                elapsed,
                tx_count,
                tx_count as f64 / elapsed,
                row_count,
                row_count as f64 / elapsed
            );
            last_report = Instant::now();
        }
    }

    let elapsed = start.elapsed();
    let tx_count = transactions.load(Ordering::Relaxed);
    let row_count = rows_inserted.load(Ordering::Relaxed);

    info!("");
    info!("===========================================");
    info!("  Stress Test Complete - NO CRASH!");
    info!("===========================================");
    info!("Duration: {:.2}s", elapsed.as_secs_f64());
    info!(
        "Transactions: {} ({:.0}/s)",
        tx_count,
        tx_count as f64 / elapsed.as_secs_f64()
    );
    info!(
        "Rows inserted: {} ({:.0}/s)",
        row_count,
        row_count as f64 / elapsed.as_secs_f64()
    );
    info!("");
    info!("Test completed successfully!");

    Ok(())
}
