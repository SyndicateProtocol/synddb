use anyhow::Result;
use clap::Parser;
use tracing::info;

use synddb_bridge_validator::{LogFormat, ValidatorConfig, ValidatorMode};

#[tokio::main]
async fn main() -> Result<()> {
    let config = ValidatorConfig::parse();

    init_logging(&config);

    info!(
        mode = ?config.mode,
        bridge_address = %config.bridge_address,
        chain_id = config.bridge_chain_id,
        "Starting bridge validator"
    );

    match config.mode {
        ValidatorMode::Primary => run_primary_validator(config).await,
        ValidatorMode::Witness => run_witness_validator(config).await,
    }
}

fn init_logging(config: &ValidatorConfig) {
    use tracing_subscriber::{fmt, EnvFilter};

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    match config.log_format {
        LogFormat::Pretty => {
            fmt().with_env_filter(filter).init();
        }
        LogFormat::Json => {
            fmt().with_env_filter(filter).json().init();
        }
    }
}

async fn run_primary_validator(_config: ValidatorConfig) -> Result<()> {
    info!("Primary validator starting...");

    // TODO: Implementation phases 2-10
    // 1. Connect to RPC
    // 2. Create BridgeClient
    // 3. Sync nonces from Bridge
    // 4. Create ValidationPipeline
    // 5. Create StoragePublisher
    // 6. Create EIP712Signer
    // 7. Start HTTP server
    // 8. Wait for shutdown

    info!("Primary validator not yet implemented");
    Ok(())
}

async fn run_witness_validator(_config: ValidatorConfig) -> Result<()> {
    info!("Witness validator starting...");

    // TODO: Witness implementation
    // 1. Subscribe to MessageInitialized events
    // 2. Fetch messages from storage
    // 3. Re-verify messages
    // 4. Sign valid messages

    info!("Witness validator not yet implemented");
    Ok(())
}
