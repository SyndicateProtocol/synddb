//! `SyndDB` Sequencer Node
//!
//! A standalone service that receives changesets from synddb-client applications,
//! assigns monotonic sequence numbers, and signs them with a private key.

use anyhow::{Context, Result};
use clap::Parser;
use std::sync::Arc;
use tokio::{net::TcpListener, signal};
use tracing::{error, info, warn};

use synddb_sequencer::{
    attestation::{AttestationConfig, AttestationVerifier},
    batcher::{Batcher, BatcherHandle},
    config::{PublisherType, SequencerConfig},
    http_api::{create_router, AppState},
    inbox::Inbox,
    publish::transport_local::{LocalTransport, LocalTransportConfig},
    signer::MessageSigner,
};
use synddb_shared::runtime;

#[tokio::main]
async fn main() -> Result<()> {
    let config = SequencerConfig::parse();
    runtime::init_logging(config.log_json);

    info!("SyndDB Sequencer starting...");

    // Log supported publisher types
    info!(
        supported = %PublisherType::supported_types().join(", "),
        selected = %config.publisher_type,
        "Publisher types"
    );

    info!(bind_address = %config.bind_address, "Configuration loaded");

    // Initialize the message signer
    let signer = MessageSigner::new(&config.signing_key)
        .context("Failed to initialize signer from SIGNING_KEY")?;

    // Log both address (for human readability) and public key (for verification)
    let pubkey_hex = format!("0x{}", hex::encode(signer.public_key()));
    info!(
        address = %signer.address(),
        public_key = %pubkey_hex,
        "Signer initialized"
    );

    let signer = Arc::new(signer);

    // Initialize CBOR batcher based on publisher_type
    let (batcher, local_transport): (Option<BatcherHandle>, Option<Arc<LocalTransport>>) =
        match config.publisher_type {
            PublisherType::None => {
                info!("Publisher disabled (messages will not be persisted)");
                (None, None)
            }
            PublisherType::Local => {
                use synddb_sequencer::publish::transport::TransportPublisher;

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
                    Arc::clone(&signer),
                );
                (Some(batcher), Some(transport))
            }
            PublisherType::Gcs => {
                #[cfg(feature = "gcs")]
                {
                    use synddb_sequencer::publish::transport_gcs::{
                        GcsTransport, GcsTransportConfig,
                    };

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
                            Arc::clone(&signer),
                        )),
                        None,
                    )
                }
                #[cfg(not(feature = "gcs"))]
                {
                    anyhow::bail!(
                        "publisher_type=gcs requires the 'gcs' feature. Compile with --features gcs"
                    );
                }
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

    // Create the inbox (shares signer with publisher)
    let inbox = Arc::new(Inbox::with_start_sequence_arc(
        Arc::clone(&signer),
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
        Some(Arc::new(AttestationVerifier::new(attestation_config)))
    } else {
        info!("Attestation verification disabled");
        None
    };

    // Create application state
    let state = AppState {
        inbox: inbox.clone(),
        attestation_verifier,
        batcher: batcher.clone(),
    };

    // Create the HTTP router
    let mut app = create_router(state);

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
        .with_graceful_shutdown(shutdown_signal(batcher, shutdown_timeout))
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
    timeout: std::time::Duration,
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
