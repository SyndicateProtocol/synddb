use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use tracing::info;

use synddb_bridge_validator::{
    bridge::BridgeClient,
    http::{handlers::AppState, start_server},
    invariants::{PriceDivergenceInvariant, PriceMetadataConsistencyInvariant},
    signing::MessageSigner,
    state::{MessageStore, NonceStore},
    storage::{providers::MemoryPublisher, StorageFetcher, StoragePublisher},
    validation::ValidationPipeline,
    LogFormat, ValidatorConfig, ValidatorMode, WitnessValidator,
};

#[cfg(feature = "gcs")]
use synddb_bridge_validator::storage::providers::GcsPublisher;

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
    let mut pipeline = ValidationPipeline::new(
        message_store,
        nonce_store,
        config.max_clock_drift(),
        config.schema_cache_ttl,
    );

    // Register price oracle invariants if enabled
    if config.enable_price_oracle_invariants {
        info!(
            max_divergence_bps = config.price_divergence_max_bps,
            "Enabling price oracle invariants"
        );
        pipeline.register_invariant(Box::new(PriceMetadataConsistencyInvariant::new()));
        pipeline.register_invariant(Box::new(PriceDivergenceInvariant::new(
            config.price_divergence_max_bps,
        )));
    }

    let pipeline = Arc::new(pipeline);

    // 6. Create storage publisher
    let storage: Arc<dyn StoragePublisher> = create_storage_publisher(&config).await?;

    // 7. Create app state
    let state = Arc::new(AppState {
        mode: config.mode,
        pipeline,
        signer,
        bridge_client,
        storage,
        api_key: config.api_key.clone(),
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

async fn run_witness_validator(config: ValidatorConfig) -> Result<()> {
    use tokio::sync::watch;

    info!("Witness validator starting...");

    // 1. Get private key
    let private_key = config
        .private_key
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("PRIVATE_KEY is required for witness validator"))?;

    // 2. Require WebSocket URL for event subscription
    if config.ws_url.is_none() {
        anyhow::bail!("WS_URL is required for witness validator to subscribe to events");
    }

    // 3. Create stores
    let message_store = Arc::new(MessageStore::new(&config.database_path)?);
    let nonce_store = Arc::new(NonceStore::new(&config.database_path)?);

    // 4. Create bridge client
    let bridge_client = Arc::new(BridgeClient::new(
        &config.rpc_url,
        config.bridge_address,
        private_key,
    )?);

    // 5. Create signer
    let signer = Arc::new(MessageSigner::new(
        private_key,
        config.bridge_chain_id,
        config.bridge_address,
    )?);

    info!(
        validator_address = %signer.address(),
        "Witness validator signer initialized"
    );

    // 6. Create validation pipeline
    let mut pipeline = ValidationPipeline::new(
        message_store.clone(),
        nonce_store,
        config.max_clock_drift(),
        config.schema_cache_ttl,
    );

    // Register price oracle invariants if enabled
    if config.enable_price_oracle_invariants {
        info!(
            max_divergence_bps = config.price_divergence_max_bps,
            "Enabling price oracle invariants"
        );
        pipeline.register_invariant(Box::new(PriceMetadataConsistencyInvariant::new()));
        pipeline.register_invariant(Box::new(PriceDivergenceInvariant::new(
            config.price_divergence_max_bps,
        )));
    }

    let pipeline = Arc::new(pipeline);

    // 7. Create storage fetcher
    let storage_fetcher = Arc::new(StorageFetcher::new());

    // 8. Create shutdown channel
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    // 9. Create witness validator
    let mut witness = WitnessValidator::new(
        config.clone(),
        bridge_client,
        signer,
        pipeline,
        storage_fetcher,
        message_store,
        shutdown_rx,
    );

    // 10. Handle shutdown signal
    let shutdown_handle = tokio::spawn(async move {
        tokio::signal::ctrl_c()
            .await
            .expect("Failed to listen for ctrl+c");
        info!("Shutdown signal received");
        let _ = shutdown_tx.send(true);
    });

    // 11. Run the witness validator
    witness.run().await?;

    shutdown_handle.abort();
    info!("Witness validator stopped");

    Ok(())
}

async fn create_storage_publisher(
    #[allow(unused_variables)] config: &ValidatorConfig,
) -> Result<Arc<dyn StoragePublisher>> {
    #[cfg(feature = "gcs")]
    if let Some(bucket) = &config.gcs_bucket {
        info!(bucket = %bucket, "Initializing GCS storage publisher");
        let publisher = GcsPublisher::new(bucket.clone()).await?;
        return Ok(Arc::new(publisher));
    }

    info!("Using in-memory storage publisher");
    Ok(Arc::new(MemoryPublisher::new()))
}
