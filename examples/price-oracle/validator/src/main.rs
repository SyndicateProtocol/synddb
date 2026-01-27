//! Price Oracle Custom Validator
//!
//! This validator extends synddb-validator with a PriceConsistencyRule that ensures
//! prices from multiple sources (CoinGecko and CoinMarketCap) agree within tolerance.
//!
//! Key insight: The validator doesn't need API keys! It only queries the database
//! where the application has already logged prices from both sources.

mod rules;

use alloy::primitives::{keccak256, B256};
use anyhow::Result;
use clap::Parser;
use std::sync::Arc;
use tokio::sync::watch;
use tracing::{debug, error, info, warn};

#[cfg(feature = "tee")]
use synddb_bootstrap::{BootstrapConfig, BootstrapStateMachine, ProverMode};

use synddb_validator::{
    bridge::{signature_store::SignatureStore, signer::BridgeSigner},
    config::{FetcherType, ValidatorConfig},
    http::{
        api::{create_router, AppState},
        signatures::{create_signature_router, SignatureApiState},
    },
    rules::RuleRegistry,
    sync::fetcher::StorageFetcher,
    validator::Validator,
};

use crate::rules::PriceConsistencyRule;

/// Price Oracle Validator configuration
///
/// Extends the base ValidatorConfig with price consistency settings
#[derive(Debug, Clone, Parser)]
#[command(name = "price-oracle-validator")]
#[command(about = "SyndDB Validator with price consistency rules")]
pub struct PriceOracleConfig {
    #[command(flatten)]
    pub base: ValidatorConfig,

    /// Maximum price difference in basis points (100 = 1%)
    #[arg(long, env = "MAX_PRICE_DIFFERENCE_BPS", default_value = "100")]
    pub max_price_difference_bps: u32,

    /// Disable price consistency rule (for testing)
    #[arg(long, env = "DISABLE_PRICE_CHECK", default_value = "false")]
    pub disable_price_check: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut config = PriceOracleConfig::parse();

    // Initialize logging
    synddb_shared::runtime::init_logging(config.base.log_json);

    // Initialize Prometheus metrics exporter
    let metrics_handle = synddb_shared::metrics::init_metrics();

    // Log supported fetcher types
    info!(
        supported = %FetcherType::supported_types().join(", "),
        selected = %config.base.fetcher_type,
        "Fetcher types"
    );

    // Resolve sequencer public key (fetch from sequencer if not provided)
    let sequencer_pubkey = config
        .base
        .resolve_sequencer_pubkey()
        .await
        .map_err(|e| anyhow::anyhow!(e))?;
    config.base.sequencer_pubkey = Some(sequencer_pubkey.clone());

    info!(
        sequencer_pubkey = %sequencer_pubkey,
        database = %config.base.database_path,
        state_db = %config.base.state_db_path,
        max_price_difference_bps = config.max_price_difference_bps,
        "Price Oracle Validator starting"
    );

    config
        .base
        .validate_bridge_config()
        .map_err(|e| anyhow::anyhow!(e))?;

    // Run TEE key bootstrapping if enabled
    #[cfg(feature = "tee")]
    if config.base.enable_key_bootstrap {
        config
            .base
            .validate_bootstrap_config()
            .map_err(|e| anyhow::anyhow!(e))?;

        info!("Starting TEE key bootstrap for price oracle validator...");

        let bootstrap_config = BootstrapConfig {
            enable_key_bootstrap: true,
            bridge_address: config.base.bridge_address.clone(),
            rpc_url: config.base.bootstrap_rpc_url.clone(),
            chain_id: config.base.bootstrap_chain_id,
            relayer_url: config.base.relayer_url.clone(),
            proof_service_url: config.base.proof_service_url.clone(),
            attestation_audience: config.base.attestation_audience.clone(),
            image_signature: config.base.image_signature.clone(),
            proof_timeout: config.base.proof_timeout,
            bootstrap_timeout: config.base.bootstrap_timeout,
            prover_mode: ProverMode::Service,
            ..Default::default()
        };

        let mut bootstrap = BootstrapStateMachine::for_validator();

        // Run bootstrap with timeout - returns the registered key
        match tokio::time::timeout(
            config.base.bootstrap_timeout,
            bootstrap.run(&bootstrap_config),
        )
        .await
        {
            Ok(Ok(key)) => {
                info!(
                    address = %key.address(),
                    "TEE key bootstrap completed successfully"
                );
                // Key is registered on-chain, validator can now proceed
            }
            Ok(Err(e)) => {
                error!(error = %e, "TEE key bootstrap failed");
                return Err(anyhow::anyhow!("Bootstrap failed: {}", e));
            }
            Err(_) => {
                error!("TEE key bootstrap timed out");
                return Err(anyhow::anyhow!(
                    "Bootstrap timed out after {:?}",
                    config.base.bootstrap_timeout
                ));
            }
        }
    }

    #[cfg(not(feature = "tee"))]
    if config.base.enable_key_bootstrap {
        return Err(anyhow::anyhow!(
            "TEE key bootstrap requires the 'tee' feature to be enabled"
        ));
    }

    // Setup shutdown channel
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let fetcher = create_fetcher(&config.base).await?;
    let mut validator = Validator::new(&config.base, fetcher.clone(), shutdown_rx.clone())?;

    // Register custom validation rules
    let mut registry = RuleRegistry::new();
    let mut price_rule = PriceConsistencyRule::new(config.max_price_difference_bps);
    if config.disable_price_check {
        info!("Price consistency check DISABLED");
        price_rule.set_enabled(false);
    } else {
        info!(
            max_difference_bps = config.max_price_difference_bps,
            "Price consistency check enabled"
        );
    }
    registry.register(Box::new(price_rule));
    validator.set_rules(registry);

    let app_state = AppState::new();
    let http_handle =
        start_http_server(app_state.clone(), config.base.bind_address, metrics_handle);

    // Setup bridge signer if enabled
    let signature_store = SignatureStore::new();
    let bridge_signer: Option<Arc<BridgeSigner>> = if config.base.is_bridge_signer() {
        let signer = BridgeSigner::new(&config.base)?;
        info!(
            signer = %signer.address(),
            bridge = %signer.bridge_contract(),
            chain_id = signer.chain_id(),
            "Bridge signer mode enabled"
        );
        Some(Arc::new(signer))
    } else {
        None
    };

    // Start signature API if bridge signer is enabled
    let signature_api_handle = bridge_signer.as_ref().map(|signer| {
        start_signature_http_server(
            signature_store.clone(),
            signer,
            config.base.bridge_signature_endpoint,
        )
    });

    // Start sync loop
    let sync_app_state = app_state.clone();
    let sync_signer = bridge_signer.clone();
    let sync_store = signature_store.clone();
    let sync_handle = tokio::spawn(async move {
        sync_app_state.set_running(true);

        // Initial sync to head
        match sync_to_head_with_signing(&mut validator, sync_signer.as_ref(), &sync_store).await {
            Ok(synced) => {
                if synced > 0 {
                    info!(synced, "Completed initial sync");
                    if let Ok(Some(seq)) = validator.last_sequence() {
                        sync_app_state.update_sync_status(Some(seq), current_timestamp());
                    }
                }
            }
            Err(e) => {
                error!(error = %e, "Initial sync failed");
            }
        }

        // Create callbacks for continuous sync
        let mut on_withdrawal = create_withdrawal_callback(sync_signer.clone(), sync_store.clone());

        let state_for_loop = sync_app_state.clone();
        let mut on_sync = move |sequence: u64| {
            state_for_loop.update_sync_status(Some(sequence), current_timestamp());
        };

        // Run continuous sync batching loop
        if let Err(e) = validator
            .run_batched(&mut on_withdrawal, &mut on_sync)
            .await
        {
            error!(error = %e, "Sync loop error");
        }

        sync_app_state.set_running(false);
    });

    // Wait for shutdown signal
    info!("Price Oracle Validator running. Press Ctrl+C to shut down.");
    synddb_shared::runtime::wait_for_shutdown().await;

    // Signal shutdown
    info!("Shutdown signal received, stopping...");
    shutdown_tx.send(true).ok();

    // Wait for sync loop with timeout
    let shutdown_timeout = config.base.shutdown_timeout;
    tokio::select! {
        _ = sync_handle => {
            info!("Sync loop stopped");
        }
        _ = tokio::time::sleep(shutdown_timeout) => {
            warn!("Shutdown timeout, forcing stop");
        }
    }

    http_handle.abort();
    if let Some(sig_handle) = signature_api_handle {
        sig_handle.abort();
    }
    info!("Price Oracle Validator stopped");
    Ok(())
}

/// Create a withdrawal signing callback
fn create_withdrawal_callback(
    signer: Option<Arc<BridgeSigner>>,
    store: SignatureStore,
) -> impl FnMut(&synddb_shared::types::payloads::WithdrawalRequest) {
    move |withdrawal: &synddb_shared::types::payloads::WithdrawalRequest| {
        if let Some(ref signer) = signer {
            let message_id = request_id_to_message_id(&withdrawal.request_id);
            match signer.sign_message_sync(message_id) {
                Ok(sig) => {
                    info!(
                        request_id = %withdrawal.request_id,
                        message_id = %sig.message_id,
                        signer = %sig.signer,
                        "Signed withdrawal message"
                    );
                    store.store(sig);
                }
                Err(e) => {
                    error!(
                        request_id = %withdrawal.request_id,
                        error = %e,
                        "Failed to sign withdrawal message"
                    );
                }
            }
        }
    }
}

/// Sync to head while signing withdrawal messages
async fn sync_to_head_with_signing(
    validator: &mut Validator,
    signer: Option<&Arc<BridgeSigner>>,
    store: &SignatureStore,
) -> Result<u64> {
    let mut next_sequence = validator.last_sequence()?.map_or(0, |s| s + 1);
    let mut synced = 0;
    let mut on_withdrawal = create_withdrawal_callback(signer.cloned(), store.clone());

    loop {
        let result = validator
            .sync_one_with_callback(next_sequence, &mut on_withdrawal)
            .await;

        match result {
            Ok(true) => {
                synced += 1;
                next_sequence += 1;
            }
            Ok(false) => break,
            Err(e) => {
                warn!(sequence = next_sequence, error = %e, "Sync error, stopping");
                break;
            }
        }
    }

    if synced > 0 {
        debug!(synced, last_sequence = next_sequence - 1, "Synced to head");
    }

    Ok(synced)
}

/// Convert a request ID string to a bytes32 message ID for the bridge
///
/// If the `request_id` is already a 0x-prefixed 32-byte hex string, parse it directly.
/// Otherwise, hash it with keccak256.
fn request_id_to_message_id(request_id: &str) -> B256 {
    // Try to parse as hex first (0x-prefixed 32-byte hex)
    if request_id.starts_with("0x") && request_id.len() == 66 {
        if let Ok(bytes) = hex::decode(&request_id[2..]) {
            if bytes.len() == 32 {
                return B256::from_slice(&bytes);
            }
        }
    }

    // Otherwise, hash the request_id
    keccak256(request_id.as_bytes())
}

/// Create the appropriate storage fetcher based on configuration
async fn create_fetcher(config: &ValidatorConfig) -> Result<Arc<dyn StorageFetcher>> {
    match config.fetcher_type {
        FetcherType::Http => {
            let url = config.sequencer_url.as_ref().ok_or_else(|| {
                anyhow::anyhow!("SEQUENCER_URL is required when fetcher_type=http")
            })?;
            info!(url = %url, "Using HTTP fetcher");
            let fetcher = synddb_validator::sync::providers::http::HttpFetcher::new(url);
            Ok(Arc::new(fetcher))
        }
        FetcherType::Gcs => {
            let bucket = config
                .gcs_bucket
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("GCS_BUCKET is required when fetcher_type=gcs"))?;
            info!(bucket = %bucket, prefix = %config.gcs_prefix, "Using GCS fetcher");
            let fetcher = synddb_validator::sync::providers::gcs::GcsFetcher::new(
                bucket.clone(),
                config.gcs_prefix.clone(),
                config.gcs_storage_emulator_host.clone(),
            )
            .await?;
            Ok(Arc::new(fetcher))
        }
    }
}

/// Get current Unix timestamp in seconds
fn current_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn start_http_server(
    app_state: AppState,
    bind_address: std::net::SocketAddr,
    metrics_handle: metrics_exporter_prometheus::PrometheusHandle,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let router = create_router(app_state, metrics_handle);
        let listener = tokio::net::TcpListener::bind(bind_address)
            .await
            .expect("Failed to bind HTTP server");
        info!(address = %bind_address, "HTTP server listening");
        axum::serve(listener, router)
            .await
            .expect("HTTP server error");
    })
}

fn start_signature_http_server(
    store: SignatureStore,
    signer: &Arc<BridgeSigner>,
    bind_address: std::net::SocketAddr,
) -> tokio::task::JoinHandle<()> {
    let sig_api_state = SignatureApiState::new(
        store,
        signer.address_formatted().to_owned(),
        signer.bridge_contract_formatted().to_owned(),
        signer.chain_id(),
    );

    tokio::spawn(async move {
        let router = create_signature_router(sig_api_state);
        let listener = tokio::net::TcpListener::bind(bind_address)
            .await
            .expect("Failed to bind signature API server");
        info!(address = %bind_address, "Signature API server listening");
        axum::serve(listener, router)
            .await
            .expect("Signature API server error");
    })
}
