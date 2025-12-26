use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use tracing::info;

use synddb_bridge_validator::{
    bridge::BridgeClient,
    http::{handlers::AppState, start_server},
    signing::MessageSigner,
    state::{MessageStore, NonceStore},
    storage::providers::MemoryPublisher,
    validation::ValidationPipeline,
    LogFormat, ValidatorConfig, ValidatorMode,
};

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

async fn run_primary_validator(config: ValidatorConfig) -> Result<()> {
    info!("Primary validator starting...");

    // 1. Get private key
    let private_key = config
        .private_key
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("PRIVATE_KEY is required for primary validator"))?;

    // 2. Create stores
    let message_store = Arc::new(MessageStore::new(&config.database_path)?);
    let nonce_store = Arc::new(NonceStore::new(&config.database_path)?);

    // 3. Create bridge client
    let bridge_client = Arc::new(BridgeClient::new(
        &config.rpc_url,
        config.bridge_address,
        private_key,
    )?);

    // 4. Create signer
    let signer = Arc::new(MessageSigner::new(
        private_key,
        config.bridge_chain_id,
        config.bridge_address,
    )?);

    info!(
        validator_address = %signer.address(),
        "Validator signer initialized"
    );

    // 5. Create validation pipeline
    let pipeline = Arc::new(ValidationPipeline::new(
        message_store,
        nonce_store,
        config.max_clock_drift(),
        config.schema_cache_ttl,
    ));

    // 6. Create storage publisher
    // TODO: Support GCS based on config
    let storage: Arc<dyn synddb_bridge_validator::storage::StoragePublisher> =
        Arc::new(MemoryPublisher::new());

    // 7. Create app state
    let state = Arc::new(AppState {
        mode: config.mode,
        pipeline,
        signer,
        bridge_client,
        storage,
    });

    // 8. Start HTTP server
    info!(
        host = %config.http_host,
        port = config.http_port,
        "Starting HTTP server"
    );

    start_server(state, &config.http_host, config.http_port).await?;

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
