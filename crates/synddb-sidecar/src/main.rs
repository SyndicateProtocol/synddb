use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;
use tracing::info;

#[derive(Parser)]
#[command(name = "synddb-sidecar")]
#[command(about = "SyndDB sidecar - monitors SQLite databases and publishes changes to DA layers", long_about = None)]
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

    info!("SyndDB Sidecar starting...");
    info!("Monitoring database: {:?}", cli.db);

    // TODO: Implement sidecar functionality based on PLAN_SIDECAR.md
    // 1. Session Monitor - attach to SQLite via Session Extension
    // 2. Batcher - accumulate changesets
    // 3. Attestor - compress and sign batches
    // 4. Publisher - publish to DA layers

    info!("Sidecar implementation coming soon!");
    info!("See PLAN_SIDECAR.md for architecture details");

    Ok(())
}
