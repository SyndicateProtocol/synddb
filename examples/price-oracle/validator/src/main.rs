//! Price Oracle Custom Validator
//!
//! This validator extends synddb-validator with a PriceConsistencyRule that ensures
//! prices from multiple sources (CoinGecko and CoinMarketCap) agree within tolerance.
//!
//! Key insight: The validator doesn't need API keys! It only queries the database
//! where the application has already logged prices from both sources.

mod rules;

use anyhow::Result;
use clap::Parser;
use std::sync::Arc;
use tokio::sync::watch;
use tracing::{debug, error, info, warn};

use synddb_validator::{
    config::{FetcherType, ValidatorConfig},
    http::api::{create_router, AppState},
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

    // Initialize metrics
    let metrics_handle = synddb_shared::metrics::init_metrics();

    // Start HTTP server for health checks
    let app_state = AppState::new();
    let http_handle =
        start_http_server(app_state.clone(), config.base.bind_address, metrics_handle);

    // Start sync loop
    let sync_app_state = app_state.clone();
    let sync_handle = tokio::spawn(async move {
        sync_app_state.set_running(true);

        // Initial sync to head
        match sync_to_head(&mut validator).await {
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
        let mut on_withdrawal = |_: &synddb_shared::types::payloads::WithdrawalRequest| {
            // No bridge signing in this example
        };

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
    info!("Price Oracle Validator stopped");
    Ok(())
}

/// Sync to the latest sequence
async fn sync_to_head(validator: &mut Validator) -> Result<u64> {
    let mut next_sequence = validator.last_sequence()?.map_or(0, |s| s + 1);
    let mut synced = 0;
    let mut on_withdrawal = |_: &synddb_shared::types::payloads::WithdrawalRequest| {};

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
