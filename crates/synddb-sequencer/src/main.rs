use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;
use tracing::info;

#[derive(Parser)]
#[command(name = "synddb-sequencer")]
#[command(about = "SyndDB sequencer - monitors SQLite databases and publishes changes to DA layers (runs as a sidecar process)", long_about = None)]
struct Cli {
    /// Path to the SQLite database to monitor
    #[arg(short, long)]
    db: PathBuf,

    /// Configuration file path
    #[arg(short, long)]
    config: Option<PathBuf>,
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

    info!("SyndDB Sequencer starting...");
    info!("Monitoring database: {:?}", cli.db);

    // TODO: Implement sequencer functionality based on PLAN_SEQUENCER.md
    // The sequencer runs as a sidecar process alongside the application
    // 1. Session Monitor - attach to SQLite via Session Extension
    // 2. Batcher - accumulate changesets
    // 3. Attestor - compress and sign batches
    // 4. Publisher - publish to DA layers

    info!("Sequencer implementation coming soon!");
    info!("See PLAN_SEQUENCER.md for architecture details");

    Ok(())
}
