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
    db: Option<PathBuf>,

    /// Configuration file path
    #[arg(short, long, default_value = "config/default.yaml")]
    config: PathBuf,
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
    info!("Loading configuration from: {:?}", cli.config);

    // Load configuration
    let mut config = synddb_sequencer::Config::from_file(&cli.config)?;

    // Override database path from CLI if provided
    if let Some(db_path) = cli.db {
        config.database.path = db_path;
    }

    info!("Monitoring database: {:?}", config.database.path);

    // Create and run sequencer
    let sequencer = synddb_sequencer::Sequencer::new(config);

    info!("Starting sequencer components...");
    info!("  - Session Monitor: Capturing changesets via SQLite Session Extension");
    info!("  - Batcher: Accumulating and batching changesets");
    info!("  - Attestor: Compressing and signing batches");
    info!("  - Publisher: Publishing to DA layers");
    info!("  - Message Monitor: Handling inbound/outbound messages");

    // Run the sequencer
    sequencer.run().await?;

    info!("Sequencer stopped");
    Ok(())
}
