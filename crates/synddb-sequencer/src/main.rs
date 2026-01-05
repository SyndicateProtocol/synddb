//! `SyndDB` Sequencer Node
//!
//! A standalone service that receives changesets from synddb-client applications,
//! assigns monotonic sequence numbers, and signs them with a private key.

use anyhow::{Context, Result};
use clap::Parser;
use std::{sync::Arc, time::Duration};
use tokio::{net::TcpListener, signal, sync::watch};
use tracing::{error, info, warn};

#[cfg(feature = "tee")]
use synddb_bootstrap::{BootstrapConfig, BootstrapStateMachine, ProverMode};

use synddb_sequencer::{
    attestation::{AttestationConfig, AttestationVerifier},
    batcher::{Batcher, BatcherHandle},
    config::{PublisherType, SequencerConfig},
    http_api::{create_router, AppState},
    inbox::Inbox,
    messages::{
        create_messages_router, MessageApiState, OutboundMonitor, OutboundMonitorConfig,
        OutboundMonitorHandle,
    },
    transport::local::{LocalTransport, LocalTransportConfig},
};
use synddb_shared::{keys::EvmKeyManager, metrics, telemetry};

#[tokio::main]
async fn main() -> Result<()> {
    let config = SequencerConfig::parse();

    // Initialize tracing with optional OpenTelemetry export and Cloud Trace correlation
    let _tracing_guard = telemetry::init_tracing(
        "synddb-sequencer",
        config.log_json,
        config.otel_enabled,
        config.gcp_project_id.clone(),
    )
    .map_err(|e| anyhow::anyhow!("Failed to initialize tracing: {e}"))?;

    // Initialize Prometheus metrics exporter
    let metrics_handle = metrics::init_metrics();

    info!("SyndDB Sequencer starting...");

    // Log supported publisher types
    info!(
        supported = %PublisherType::supported_types().join(", "),
        selected = %config.publisher_type,
        "Publisher types"
    );

    info!(bind_address = %config.bind_address, "Configuration loaded");

    // Generate signing key - either via bootstrap or directly
    let key_manager: Arc<EvmKeyManager> = {
        #[cfg(feature = "tee")]
        if config.enable_key_bootstrap {
            config
                .validate_bootstrap_config()
                .map_err(|e| anyhow::anyhow!(e))?;

            info!("Starting TEE key bootstrap...");

            let bootstrap_config = BootstrapConfig {
                enable_key_bootstrap: true,
                bridge_address: config.bridge_address.clone(),
                rpc_url: config.bootstrap_rpc_url.clone(),
                chain_id: config.bootstrap_chain_id,
                relayer_url: config.relayer_url.clone(),
                proof_service_url: config.proof_service_url.clone(),
                attestation_audience: config.attestation_audience.clone(),
                cosign_signature: config.cosign_signature.clone(),
                cosign_pubkey: config.cosign_pubkey.clone(),
                proof_timeout: config.proof_timeout,
                bootstrap_timeout: config.bootstrap_timeout,
                prover_mode: ProverMode::Service,
                ..Default::default()
            };

            let mut bootstrap = BootstrapStateMachine::for_sequencer();

            // Run bootstrap with timeout - returns the registered key
            match tokio::time::timeout(config.bootstrap_timeout, bootstrap.run(&bootstrap_config))
                .await
            {
                Ok(Ok(key)) => {
                    info!(
                        address = %key.address(),
                        "TEE key bootstrap completed successfully"
                    );
                    key
                }
                Ok(Err(e)) => {
                    error!(error = %e, "TEE key bootstrap failed");
                    return Err(anyhow::anyhow!("Bootstrap failed: {}", e));
                }
                Err(_) => {
                    error!("TEE key bootstrap timed out");
                    return Err(anyhow::anyhow!(
                        "Bootstrap timed out after {:?}",
                        config.bootstrap_timeout
                    ));
                }
            }
        } else {
            // Generate key without bootstrap
            Arc::new(EvmKeyManager::generate())
        }

        #[cfg(not(feature = "tee"))]
        {
            if config.enable_key_bootstrap {
                return Err(anyhow::anyhow!(
                    "TEE key bootstrap requires the 'tee' feature to be enabled"
                ));
            }
            Arc::new(EvmKeyManager::generate())
        }
    };

    // Log only the public key and address (safe to expose)
    let pubkey_hex = format!("0x{}", hex::encode(key_manager.public_key()));
    info!(
        address = %key_manager.address(),
        public_key = %pubkey_hex,
        "TEE signing key ready"
    );

    // Initialize CBOR batcher based on publisher_type
    let (batcher, local_transport): (Option<BatcherHandle>, Option<Arc<LocalTransport>>) =
        match config.publisher_type {
            PublisherType::None => {
                warn!(
                    "Publisher is DISABLED - messages will NOT be persisted! \
                    Set PUBLISHER_TYPE=local or PUBLISHER_TYPE=gcs for production."
                );
                (None, None)
            }
            PublisherType::Local => {
                use synddb_sequencer::transport::traits::TransportPublisher;

                // Use optional LOCAL_STORAGE_PATH for file persistence, else default to in-memory
                let transport_config = config.local_storage_path.as_ref().map_or_else(
                    || {
                        info!("Using in-memory storage for local transport");
                        LocalTransportConfig::in_memory()
                    },
                    |path| {
                        info!(path = %path, "Using SQLite storage for local transport");
                        LocalTransportConfig::file(path)
                    },
                );

                let transport = Arc::new(LocalTransport::new(transport_config));
                let batch_config = config.batch_config();
                info!(
                    max_messages = batch_config.max_messages,
                    max_bytes = batch_config.max_batch_bytes,
                    flush_interval_ms = batch_config.flush_interval.as_millis(),
                    "Initializing CBOR batcher with local transport"
                );

                let batcher = Batcher::spawn(
                    batch_config,
                    Arc::clone(&transport) as Arc<dyn TransportPublisher>,
                    Arc::clone(&key_manager),
                );
                (Some(batcher), Some(transport))
            }
            PublisherType::Gcs => {
                use synddb_sequencer::transport::gcs::{GcsTransport, GcsTransportConfig};

                let bucket = config.gcs_bucket.as_ref().ok_or_else(|| {
                    anyhow::anyhow!("GCS_BUCKET is required when publisher_type=gcs")
                })?;

                let mut transport_config =
                    GcsTransportConfig::new(bucket).with_prefix(&config.gcs_prefix);
                if let Some(ref emulator_host) = config.gcs_storage_emulator_host {
                    transport_config = transport_config.with_emulator_host(emulator_host);
                }

                let transport = GcsTransport::new(transport_config)
                    .await
                    .context("Failed to initialize GCS transport")?;

                let batch_config = config.batch_config();
                info!(
                    max_messages = batch_config.max_messages,
                    max_bytes = batch_config.max_batch_bytes,
                    flush_interval_ms = batch_config.flush_interval.as_millis(),
                    "Initializing CBOR batcher with GCS transport"
                );

                (
                    Some(Batcher::spawn(
                        batch_config,
                        Arc::new(transport),
                        Arc::clone(&key_manager),
                    )),
                    None,
                )
            }
        };

    // Try to recover sequence from local transport
    let start_sequence = local_transport.as_ref().map_or(0, |transport| {
        transport.latest_sequence().map_or_else(
            || {
                info!("No previous state found, starting from sequence 0");
                0
            },
            |seq| {
                info!(sequence = seq, "Recovered sequence from local storage");
                seq + 1 // Start from next sequence
            },
        )
    });

    // Create the inbox (shares key manager with batcher)
    let inbox = Arc::new(Inbox::with_start_sequence(
        Arc::clone(&key_manager),
        start_sequence,
    ));

    // Initialize attestation verifier if configured
    let attestation_verifier = if config.verify_attestation {
        let attestation_config = AttestationConfig {
            expected_audience: config
                .attestation_service_url
                .clone()
                .unwrap_or_else(|| format!("http://{}", config.bind_address)),
            verify_tee_claims: true,
            expected_image_digest: None,
        };

        info!("Attestation verification enabled");
        Some(Arc::new(AttestationVerifier::new(attestation_config)?))
    } else {
        warn!(
            "Attestation verification is DISABLED. \
            In production, set VERIFY_ATTESTATION=true to verify client TEE tokens."
        );
        None
    };

    // Create application state
    let state = AppState {
        inbox: inbox.clone(),
        attestation_verifier,
        batcher: batcher.clone(),
    };

    // Create the HTTP router
    let mut app = create_router(state, metrics_handle);

    // Create shutdown channel for background tasks
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    // Initialize outbound message monitor if APP_DATABASE_PATH is set
    let outbound_handle: Option<OutboundMonitorHandle> =
        if let Some(ref db_path) = config.app_database_path {
            info!(
                db_path = %db_path,
                poll_interval_ms = config.outbound_poll_interval_ms,
                "Starting outbound message monitor"
            );

            let monitor_config = OutboundMonitorConfig {
                db_path: db_path.clone(),
                poll_interval: Duration::from_millis(config.outbound_poll_interval_ms),
                batch_size: 100,
            };

            let monitor = OutboundMonitor::new(monitor_config);
            let handle = OutboundMonitorHandle::new(monitor.tracker());

            // Spawn the monitor task
            let monitor_shutdown = shutdown_rx.clone();
            tokio::spawn(async move {
                monitor.run(monitor_shutdown).await;
            });

            Some(handle)
        } else {
            info!("Outbound message monitor disabled (APP_DATABASE_PATH not set)");
            None
        };

    // Mount messages API for inbound/outbound message passing
    let mut message_state = MessageApiState::new();
    if let Some(handle) = outbound_handle {
        message_state = message_state.with_outbound(handle);
    }
    app = app.merge(create_messages_router(message_state));

    // Mount storage fetch API if local transport is configured
    if let Some(ref transport) = local_transport {
        info!("Mounting local storage fetch API at /storage/*");
        app = app.nest("/storage", Arc::clone(transport).routes());
    }

    // Bind and serve
    info!(address = %config.bind_address, "Sequencer listening");
    let listener = TcpListener::bind(&config.bind_address)
        .await
        .context(format!("Failed to bind to {}", config.bind_address))?;

    // Run server with graceful shutdown
    let shutdown_timeout = config.shutdown_timeout;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal(batcher, shutdown_tx, shutdown_timeout))
        .await
        .map_err(|e| {
            error!(error = %e, "Server error");
            e
        })
        .context("Server failed")?;

    info!("Sequencer shutdown complete");
    Ok(())
}

/// Wait for shutdown signal and perform graceful shutdown
async fn shutdown_signal(
    batcher: Option<BatcherHandle>,
    shutdown_tx: watch::Sender<bool>,
    timeout: Duration,
) {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("Failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {
            info!("Received Ctrl+C, initiating graceful shutdown");
        }
        _ = terminate => {
            info!("Received SIGTERM, initiating graceful shutdown");
        }
    }

    // Signal background tasks to shutdown
    let _ = shutdown_tx.send(true);

    // Flush batcher before shutdown
    if let Some(batcher) = batcher {
        info!("Flushing batcher before shutdown");
        match tokio::time::timeout(timeout, batcher.shutdown()).await {
            Ok(Ok(())) => info!("Batcher shutdown complete"),
            Ok(Err(e)) => warn!(error = %e, "Batcher shutdown failed"),
            Err(_) => warn!("Batcher shutdown timed out"),
        }
    }

    info!("Graceful shutdown complete");
}
