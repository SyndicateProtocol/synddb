use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;
use tracing::info;

#[derive(Parser)]
#[command(name = "synddb-sequencer")]
#[command(about = "SyndDB sequencer - receives changesets from client libraries and publishes to DA layers", long_about = None)]
struct Cli {
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
    let config = synddb_sequencer::Config::from_file(&cli.config)?;

    // Create and run sequencer
    let sequencer = synddb_sequencer::Sequencer::new(config);

    info!("Starting sequencer components...");
    info!("  - HTTP Receiver: Receiving changesets from client libraries");
    info!("  - Batcher: Accumulating and batching changesets");
    info!("  - Attestor: Compressing and signing batches");
    info!("  - Publisher: Publishing to DA layers");
    info!("  - Message Monitor: Handling inbound/outbound messages");

    // Run the sequencer
    sequencer.run().await?;

    info!("Sequencer stopped");
    Ok(())
}
