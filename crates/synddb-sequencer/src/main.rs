//! `SyndDB` Sequencer Node
//!
//! A standalone service that receives changesets from synddb-client applications,
//! assigns monotonic sequence numbers, and signs them with a private key.

use anyhow::{Context, Result};
use clap::Parser;
use std::sync::Arc;
use tokio::{net::TcpListener, signal};
use tracing::{error, info, warn};

use synddb_sequencer::{attestation::{AttestationConfig, AttestationVerifier}, batcher::{ BatcherHandle}, config::{PublisherType, SequencerConfig}, http_api::{create_router, AppState}, inbox::Inbox, publish::{
    local::{LocalConfig, LocalPublisher},
    traits::StoragePublisher,
    transport_local::LocalTransport,
}, signer::MessageSigner};
use synddb_sequencer::batcher::Batcher;
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

    // Initialize legacy publisher based on publisher_type
    // Note: LocalCbor and Gcs use the CBOR batcher instead
    let (publisher, local_publisher): (
        Option<Arc<dyn StoragePublisher>>,
        Option<Arc<LocalPublisher>>,
    ) = match config.publisher_type {
        PublisherType::None | PublisherType::LocalCbor => {
            // LocalCbor uses CBOR batcher with LocalTransport (initialized below),
            // not the legacy JSON publisher
            if matches!(config.publisher_type, PublisherType::None) {
                info!("Publisher disabled (messages will not be persisted)");
            }
            (None, None)
        }
        PublisherType::Local => {
            let path = config.local_storage_path.as_ref().ok_or_else(|| {
                anyhow::anyhow!("LOCAL_STORAGE_PATH is required when publisher_type=local")
            })?;
            let local_config = if path == ":memory:" {
                LocalConfig::in_memory()
            } else {
                LocalConfig::file(path)
            };
            let local_pub = LocalPublisher::new_arc(local_config, Arc::clone(&signer))
                .context("Failed to initialize local publisher")?;
            (
                Some(local_pub.clone() as Arc<dyn StoragePublisher>),
                Some(local_pub),
            )
        }
        PublisherType::Gcs => {
            #[cfg(feature = "gcs")]
            {
                // GCS now uses CBOR batching via the Batcher, no legacy JSON publisher needed
                (None, None)
            }
            #[cfg(not(feature = "gcs"))]
            {
                anyhow::bail!(
                    "publisher_type=gcs requires the 'gcs' feature. Compile with --features gcs"
                );
            }
        }
    };

    // Initialize CBOR batcher for LocalCbor or GCS transport
    let (batcher, local_transport): (Option<BatcherHandle>, Option<Arc<LocalTransport>>) =
        match config.publisher_type {
            PublisherType::LocalCbor => {
                use synddb_sequencer::publish::transport::TransportPublisher;

                let transport = Arc::new(LocalTransport::new());
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
                    (None, None)
                }
            }
            _ => (None, None),
        };

    // Try to recover sequence from publisher or batcher
    let start_sequence = match &publisher {
        Some(pub_) => match pub_.load_state().await {
            Ok(Some(seq)) => {
                info!(sequence = seq, "Recovered sequence from publisher");
                seq + 1 // Start from next sequence
            }
            Ok(None) => {
                info!("No previous state found, starting from sequence 0");
                0
            }
            Err(e) => {
                warn!(error = %e, "Failed to load state, starting from 0");
                0
            }
        },
        None => 0,
    };

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
        publisher: publisher.clone(),
        attestation_verifier,
        batcher: batcher.clone(),
    };

    // Create the HTTP router
    let mut app = create_router(state);

    // Mount storage fetch API if configured
    // - LocalPublisher: JSON format (legacy)
    // - LocalTransport: CBOR format with COSE signatures
    if let Some(ref local_pub) = local_publisher {
        info!("Mounting local storage fetch API at /storage/* (JSON format)");
        app = app.nest("/storage", local_pub.routes());
    } else if let Some(ref transport) = local_transport {
        info!("Mounting local storage fetch API at /storage/* (CBOR format)");
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
        .with_graceful_shutdown(shutdown_signal(inbox, publisher, batcher, shutdown_timeout))
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
    inbox: Arc<Inbox>,
    publisher: Option<Arc<dyn StoragePublisher>>,
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

    // Save state before shutdown (for legacy publisher)
    if let Some(pub_) = publisher {
        let current_seq = inbox.current_sequence();
        if current_seq > 0 {
            info!(sequence = current_seq - 1, "Saving state before shutdown");
            match tokio::time::timeout(timeout, pub_.save_state(current_seq - 1)).await {
                Ok(Ok(())) => info!("State saved successfully"),
                Ok(Err(e)) => warn!(error = %e, "Failed to save state"),
                Err(_) => warn!("State save timed out"),
            }
        }
    }

    info!("Graceful shutdown complete");
}
