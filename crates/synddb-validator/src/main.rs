//! SyndDB Validator binary entry point

use anyhow::Result;
use clap::Parser;
use tracing::info;

use synddb_validator::ValidatorConfig;

fn main() -> Result<()> {
    let config = ValidatorConfig::parse();
    init_logging(&config);

    info!("SyndDB Validator starting...");
    info!(sequencer = %config.sequencer_address, "Configuration loaded");
    info!(database = %config.database_path, "Database path");

    if let Some(bucket) = &config.gcs_bucket {
        info!(bucket = %bucket, prefix = %config.gcs_prefix, "GCS configured");
    } else {
        info!("GCS not configured - no DA fetcher available");
    }

    // TODO: Initialize components and run sync loop
    info!("Validator skeleton ready - implementation in progress");

    Ok(())
}

fn init_logging(config: &ValidatorConfig) {
    use tracing_subscriber::{filter::LevelFilter, prelude::*, EnvFilter};

    let env_filter = EnvFilter::builder()
        .with_default_directive(LevelFilter::INFO.into())
        .from_env_lossy();

    if config.log_json {
        tracing_subscriber::registry()
            .with(env_filter)
            .with(tracing_subscriber::fmt::layer().json())
            .init();
    } else {
        tracing_subscriber::registry()
            .with(env_filter)
            .with(tracing_subscriber::fmt::layer().with_target(true))
            .init();
    }
}
