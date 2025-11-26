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

use synddb_sequencer::{
    config::SequencerConfig,
    http_api::{create_router, AppState},
    inbox::Inbox,
    publish::DAPublisher,
    signer::MessageSigner,
};

/// Initialize logging based on configuration
fn init_logging(config: &SequencerConfig) {
    use tracing_subscriber::prelude::*;

    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        tracing_subscriber::EnvFilter::new(format!("synddb_sequencer={}", config.log_level))
    });

    if config.log_json {
        // JSON format for production
        tracing_subscriber::registry()
            .with(env_filter)
            .with(tracing_subscriber::fmt::layer().json())
            .init();
    } else {
        // Pretty format for development
        tracing_subscriber::registry()
            .with(env_filter)
            .with(tracing_subscriber::fmt::layer().with_target(true))
            .init();
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Parse configuration from CLI args and environment
    let config = SequencerConfig::parse();

    // Initialize logging
    init_logging(&config);

    info!("SyndDB Sequencer starting...");
    info!(bind_address = %config.bind_address, "Configuration loaded");

    // Initialize the message signer
    let signer = MessageSigner::new(&config.signing_key)
        .context("Failed to initialize signer from SIGNING_KEY")?;

    info!(signer_address = %format!("{:?}", signer.address()), "Signer initialized");

    // Initialize publisher if GCS is configured
    let publisher: Option<Arc<dyn DAPublisher>> = config.gcs_bucket.as_ref().map_or_else(
        || {
            info!("No publisher configured (messages will not be persisted)");
            None
        },
        |_bucket| {
            warn!(
                "GCS_BUCKET specified but `gcs` feature not enabled. Compile with `--features gcs`"
            );
            None
        },
    );

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

    // Create the inbox
    let inbox = Arc::new(Inbox::with_start_sequence(signer, start_sequence));

    // Initialize attestation verifier if configured
    let attestation_verifier = if config.verify_attestation {
        use synddb_sequencer::attestation::{AttestationConfig, AttestationVerifier};

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
    let app = create_router(state);

    // Bind and serve
    let listener = TcpListener::bind(&config.bind_address)
        .await
        .context(format!("Failed to bind to {}", config.bind_address))?;

    info!(address = %config.bind_address, "Sequencer listening");
    info!("Endpoints:");
    info!("  POST /changesets       - Submit changeset batch");
    info!("  POST /withdrawals      - Submit withdrawal request");
    info!("  GET  /messages/:seq    - Retrieve message by sequence");
    info!("  GET  /health           - Health check (liveness)");
    info!("  GET  /ready            - Readiness check");
    info!("  GET  /status           - Sequencer status");

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
