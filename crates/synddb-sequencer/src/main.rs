//! `SyndDB` Sequencer Node
//!
//! A standalone service that receives changesets from synddb-client applications,
//! assigns monotonic sequence numbers, and signs them with a private key.

use anyhow::{Context, Result};
use clap::Parser;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::signal;
use tracing::{error, info, warn};

use synddb_sequencer::attestation::{AttestationConfig, AttestationVerifier};
use synddb_sequencer::config::PublisherType;
use synddb_sequencer::publish::local::{LocalConfig, LocalPublisher};
use synddb_sequencer::publish::traits::DAPublisher;
use synddb_sequencer::{
    config::SequencerConfig,
    http_api::{create_router, AppState},
    inbox::Inbox,
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

    info!(signer_address = %format!("{:?}", signer.address()), "Signer initialized");

    let signer = Arc::new(signer);

    // Initialize publisher based on publisher_type
    let (publisher, local_publisher): (Option<Arc<dyn DAPublisher>>, Option<Arc<LocalPublisher>>) =
        match config.publisher_type {
            PublisherType::None => {
                info!("Publisher disabled (messages will not be persisted)");
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
                    Some(local_pub.clone() as Arc<dyn DAPublisher>),
                    Some(local_pub),
                )
            }
            PublisherType::Gcs => {
                #[cfg(feature = "gcs")]
                {
                    use synddb_sequencer::publish::gcs::{GcsConfig, GcsPublisher};
                    let bucket = config.gcs_bucket.as_ref().ok_or_else(|| {
                        anyhow::anyhow!("GCS_BUCKET is required when publisher_type=gcs")
                    })?;
                    let gcs_config = GcsConfig::new(bucket).with_prefix(&config.gcs_prefix);
                    let gcs_pub = GcsPublisher::new(gcs_config, Arc::clone(&signer))
                        .await
                        .context("Failed to initialize GCS publisher")?;
                    (Some(Arc::new(gcs_pub) as Arc<dyn DAPublisher>), None)
                }
                #[cfg(not(feature = "gcs"))]
                {
                    anyhow::bail!(
                        "publisher_type=gcs requires the 'gcs' feature. Compile with --features gcs"
                    );
                }
            }
        };

    // Try to recover sequence from publisher
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
    };

    // Create the HTTP router
    let mut app = create_router(state);

    // Mount local publisher's DA fetch API if configured
    if let Some(ref local_pub) = local_publisher {
        info!("Mounting local DA fetch API at /da/*");
        app = app.nest("/da", local_pub.routes());
    }

    // Bind and serve
    info!(address = %config.bind_address, "Sequencer listening");
    let listener = TcpListener::bind(&config.bind_address)
        .await
        .context(format!("Failed to bind to {}", config.bind_address))?;

    // Run server with graceful shutdown
    let shutdown_timeout = config.shutdown_timeout;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal(inbox, publisher, shutdown_timeout))
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
    publisher: Option<Arc<dyn DAPublisher>>,
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

    // Save state before shutdown
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
