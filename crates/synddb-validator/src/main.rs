//! `SyndDB` Validator binary entry point

use alloy::primitives::{keccak256, B256};
use anyhow::Result;
use clap::Parser;
use std::sync::Arc;
use tokio::sync::watch;
use tracing::{debug, error, info, warn};

use synddb_shared::runtime;
use synddb_validator::bridge::signature_store::SignatureStore;
use synddb_validator::bridge::signer::BridgeSigner;
use synddb_validator::config::{FetcherType, ValidatorConfig};
use synddb_validator::http::api::{create_router, AppState};
use synddb_validator::http::signatures::{create_signature_router, SignatureApiState};
use synddb_validator::sync::fetcher::StorageFetcher;
use synddb_validator::validator::Validator;

#[tokio::main]
async fn main() -> Result<()> {
    let config = ValidatorConfig::parse();
    runtime::init_logging(config.log_json);

    info!(
        sequencer = %config.sequencer_address,
        database = %config.database_path,
        state_db = %config.state_db_path,
        "`SyndDB` Validator starting"
    );

    // Log supported fetcher types
    info!(
        supported = %FetcherType::supported_types().join(", "),
        selected = %config.fetcher_type,
        "Fetcher types"
    );

    config
        .validate_bridge_config()
        .map_err(|e| anyhow::anyhow!(e))?;

    // Using watch::channel for shutdown because we need broadcast semantics:
    // the shutdown signal must be received by multiple tasks (sync loop, validator internals).
    // mpsc::channel is single-consumer and wouldn't allow cloning the receiver.
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let fetcher = create_fetcher(&config).await?;
    let mut validator = Validator::new(&config, fetcher.clone(), shutdown_rx.clone())?;
    let app_state = AppState::new();

    let http_handle = start_http_server(app_state.clone(), config.bind_address);

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

    let signature_api_handle = bridge_signer.as_ref().map(|signer| {
        start_signature_http_server(
            signature_store.clone(),
            signer,
            config.bridge_signature_endpoint,
        )
    });

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
    runtime::wait_for_shutdown().await;

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
                config.storage_emulator_host.clone(),
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
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let router = create_router(app_state);
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
