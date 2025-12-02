//! Runtime utilities shared across `SyndDB` binaries
//!
//! Provides common functionality for logging initialization and shutdown signal handling.

use tracing_subscriber::{filter::LevelFilter, prelude::*, EnvFilter};

/// Initialize tracing-subscriber with JSON or pretty format
///
/// Reads `RUST_LOG` environment variable for filtering, defaulting to INFO level.
pub fn init_logging(log_json: bool) {
    let env_filter = EnvFilter::builder()
        .with_default_directive(LevelFilter::INFO.into())
        .from_env_lossy();

    if log_json {
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

/// Wait for shutdown signal (Ctrl+C or SIGTERM)
///
/// Returns when either signal is received.
pub async fn wait_for_shutdown() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
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
