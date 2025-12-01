//! `SyndDB` Validator binary entry point

use alloy::primitives::{keccak256, B256};
use anyhow::Result;
use clap::Parser;
use std::sync::Arc;
use tokio::signal;
use tokio::sync::watch;
use tracing::{debug, error, info, warn};

use synddb_validator::{
    create_router, create_signature_router, AppState, BridgeSigner, SignatureApiState,
    SignatureStore, Validator, ValidatorConfig,
};

#[tokio::main]
async fn main() -> Result<()> {
    let config = ValidatorConfig::parse();
    init_logging(&config);

    info!("`SyndDB` Validator starting...");
    info!(sequencer = %config.sequencer_address, "Configuration loaded");
    info!(database = %config.database_path, "Database path");
    info!(state_db = %config.state_db_path, "State database path");

    // Validate bridge config if enabled
    config
        .validate_bridge_config()
        .map_err(|e| anyhow::anyhow!(e))?;

    // Create shutdown channel
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    // Create DA fetcher based on config
    let fetcher = create_fetcher(&config).await?;

    // Create validator
    let mut validator = Validator::new(&config, fetcher.clone(), shutdown_rx.clone())?;

    // Create HTTP app state
    let app_state = AppState::new();

    // Get initial sync state
    if let Ok(Some(seq)) = validator.last_sequence() {
        info!(last_sequence = seq, "Resuming from previous state");
    } else {
        info!("Starting fresh sync from sequence 0");
    }

    // Start HTTP server
    let http_app_state = app_state.clone();
    let bind_address = config.bind_address;
    let http_handle = tokio::spawn(async move {
        let router = create_router(http_app_state);
        let listener = tokio::net::TcpListener::bind(bind_address)
            .await
            .expect("Failed to bind HTTP server");
        info!(address = %bind_address, "HTTP server listening");

        axum::serve(listener, router)
            .await
            .expect("HTTP server error");
    });

    // Create bridge signer and signature store if enabled
    let signature_store = SignatureStore::new();
    let bridge_signer: Option<Arc<BridgeSigner>> = if config.is_bridge_signer() {
        let signer = BridgeSigner::new(&config)?;
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

    // Start signature API server if bridge signer is enabled
    let signature_api_handle = if let Some(ref signer) = bridge_signer {
        let sig_api_state = SignatureApiState::new(
            signature_store.clone(),
            format!("{:#x}", signer.address()),
            format!("{:#x}", signer.bridge_contract()),
            signer.chain_id(),
        );

        let sig_bind_address = config.bridge_signature_endpoint;
        Some(tokio::spawn(async move {
            let router = create_signature_router(sig_api_state);
            let listener = tokio::net::TcpListener::bind(sig_bind_address)
                .await
                .expect("Failed to bind signature API server");
            info!(address = %sig_bind_address, "Signature API server listening");

            axum::serve(listener, router)
                .await
                .expect("Signature API server error");
        }))
    } else {
        None
    };

    // Start sync loop
    let sync_app_state = app_state.clone();
    let sync_signer = bridge_signer.clone();
    let sync_store = signature_store.clone();
    let sync_handle = tokio::spawn(async move {
        // Mark as running
        sync_app_state.set_running(true);

        // Initial sync to head
        match sync_to_head_with_signing(&mut validator, sync_signer.as_ref(), &sync_store).await {
            Ok(synced) => {
                if synced > 0 {
                    info!(synced, "Completed initial sync");
                    if let Ok(Some(seq)) = validator.last_sequence() {
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap()
                            .as_secs();
                        sync_app_state.update_sync_status(Some(seq), now);
                    }
                }
            }
            Err(e) => {
                error!(error = %e, "Initial sync failed");
            }
        }

        // Create callbacks for continuous sync
        let signer_for_loop = sync_signer.clone();
        let store_for_loop = sync_store.clone();
        let state_for_loop = sync_app_state.clone();

        // Withdrawal signing callback
        let mut on_withdrawal = move |withdrawal: &synddb_shared::types::WithdrawalRequest| {
            if let Some(ref signer) = signer_for_loop {
                let message_id = request_id_to_message_id(&withdrawal.request_id);
                match signer.sign_message_sync(message_id) {
                    Ok(sig) => {
                        info!(
                            request_id = %withdrawal.request_id,
                            message_id = %sig.message_id,
                            signer = %sig.signer,
                            "Signed withdrawal message"
                        );
                        store_for_loop.store(sig);
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
        };

        // Sync progress callback - updates HTTP status
        let mut on_sync = move |sequence: u64| {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();
            state_for_loop.update_sync_status(Some(sequence), now);
        };

        // Run continuous sync loop with callbacks
        if let Err(e) = validator
            .run_with_callbacks(&mut on_withdrawal, &mut on_sync)
            .await
        {
            error!(error = %e, "Sync loop error");
        }

        sync_app_state.set_running(false);
    });

    // Wait for shutdown signal
    info!("Validator running. Press Ctrl+C to shut down.");
    wait_for_shutdown().await;

    // Signal shutdown
    info!("Shutdown signal received, stopping...");
    shutdown_tx.send(true).ok();

    // Wait for sync loop to finish (with timeout)
    let shutdown_timeout = config.shutdown_timeout;
    tokio::select! {
        _ = sync_handle => {
            info!("Sync loop stopped");
        }
        _ = tokio::time::sleep(shutdown_timeout) => {
            warn!("Shutdown timeout, forcing stop");
        }
    }

    // HTTP server will be dropped when we exit
    http_handle.abort();
    if let Some(sig_handle) = signature_api_handle {
        sig_handle.abort();
    }

    info!("`SyndDB` Validator stopped");
    Ok(())
}

/// Sync to head while signing withdrawal messages
async fn sync_to_head_with_signing(
    validator: &mut Validator,
    signer: Option<&Arc<BridgeSigner>>,
    store: &SignatureStore,
) -> Result<u64> {
    let mut next_sequence = validator.last_sequence()?.map_or(0, |s| s + 1);
    let mut synced = 0;

    // Create the withdrawal callback
    let signer_clone = signer.cloned();
    let store_clone = store.clone();
    let mut on_withdrawal = move |withdrawal: &synddb_shared::types::WithdrawalRequest| {
        if let Some(ref signer) = signer_clone {
            // Convert request_id to message_id (bytes32)
            // The bridge expects keccak256(request_id) as the message ID
            let message_id = request_id_to_message_id(&withdrawal.request_id);

            // Sign the message
            match signer.sign_message_sync(message_id) {
                Ok(sig) => {
                    info!(
                        request_id = %withdrawal.request_id,
                        message_id = %sig.message_id,
                        signer = %sig.signer,
                        "Signed withdrawal message"
                    );
                    store_clone.store(sig);
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
    };

    loop {
        // Sync with callback for withdrawals
        let result = validator
            .sync_one_with_callback(next_sequence, &mut on_withdrawal)
            .await;

        match result {
            Ok(true) => {
                synced += 1;
                next_sequence += 1;
            }
            Ok(false) => {
                // Caught up to head
                break;
            }
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

/// Create the appropriate DA fetcher based on configuration
async fn create_fetcher(config: &ValidatorConfig) -> Result<Arc<dyn synddb_validator::DAFetcher>> {
    #[cfg(feature = "gcs")]
    if let Some(bucket) = &config.gcs_bucket {
        info!(bucket = %bucket, prefix = %config.gcs_prefix, "Using GCS fetcher");
        let fetcher = synddb_validator::sync::providers::GcsFetcher::new(
            bucket.clone(),
            config.gcs_prefix.clone(),
        )
        .await?;
        return Ok(Arc::new(fetcher));
    }

    // No fetcher configured
    anyhow::bail!(
        "No DA fetcher configured. Set GCS_BUCKET environment variable or --gcs-bucket flag."
    );
}

/// Wait for shutdown signal (Ctrl+C or SIGTERM)
async fn wait_for_shutdown() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("Failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
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
