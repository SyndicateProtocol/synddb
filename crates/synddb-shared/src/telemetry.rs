//! OpenTelemetry integration for distributed tracing
//!
//! This module provides utilities for integrating with Google Cloud Trace
//! via OpenTelemetry. When the `otel` feature is enabled, traces are exported
//! to an OTLP endpoint (typically the OpenTelemetry Collector or Cloud Trace).
//!
//! # Configuration
//!
//! The following environment variables control the tracing behavior:
//!
//! | Variable | Description | Default |
//! |----------|-------------|---------|
//! | `OTEL_EXPORTER_OTLP_ENDPOINT` | OTLP collector endpoint | `http://localhost:4317` |
//! | `OTEL_SERVICE_NAME` | Service name in traces | Required |
//! | `GCP_PROJECT_ID` | GCP project for trace correlation | Required for Cloud Trace |
//! | `RUST_LOG` | Log level filter | `info` |
//!
//! # Google Cloud Trace Integration
//!
//! When `otel` is enabled and `gcp_project_id` is provided, logs are formatted
//! for Cloud Logging with automatic trace correlation. The JSON output includes
//! `logging.googleapis.com/trace` and `logging.googleapis.com/spanId` fields
//! that link logs to their corresponding traces in Cloud Trace.
//!
//! # Usage
//!
//! ```rust,ignore
//! use synddb_shared::telemetry;
//!
//! // Initialize with OTLP export and Cloud Trace correlation
//! let _guard = telemetry::init_tracing(
//!     "my-service",
//!     true,  // log_json
//!     true,  // enable_otel
//!     Some("my-gcp-project".to_string()),
//! )?;
//!
//! // Your application code with spans...
//!
//! // Guard is dropped on shutdown, flushing traces
//! ```

#[cfg(feature = "otel")]
use opentelemetry::trace::TracerProvider as _;
#[cfg(feature = "otel")]
use opentelemetry_otlp::WithExportConfig;
use tracing_subscriber::{filter::LevelFilter, prelude::*, EnvFilter};

/// Guard that ensures OpenTelemetry shutdown on drop
///
/// When this guard is dropped, it flushes and shuts down the OpenTelemetry
/// tracer provider, ensuring all pending traces are exported.
#[derive(Debug)]
pub struct TracingGuard {
    #[cfg(feature = "otel")]
    _provider: Option<opentelemetry_sdk::trace::SdkTracerProvider>,
}

impl Drop for TracingGuard {
    fn drop(&mut self) {
        #[cfg(feature = "otel")]
        if let Some(ref provider) = self._provider {
            if let Err(e) = provider.shutdown() {
                eprintln!("Error shutting down tracer provider: {e:?}");
            }
        }
    }
}

/// Initialize tracing with optional OpenTelemetry export and Cloud Trace correlation
///
/// # Arguments
///
/// * `service_name` - The name of the service for trace attribution
/// * `log_json` - Whether to output logs as JSON (for Cloud Logging)
/// * `enable_otel` - Whether to enable OpenTelemetry trace export
/// * `gcp_project_id` - GCP project ID for Cloud Trace log correlation (optional)
///
/// # Returns
///
/// A guard that must be held for the lifetime of the application.
/// Dropping the guard flushes pending traces and shuts down the exporter.
///
/// # Errors
///
/// Returns an error if OpenTelemetry initialization fails (e.g., invalid endpoint).
pub fn init_tracing(
    #[allow(unused_variables)] service_name: &str,
    log_json: bool,
    enable_otel: bool,
    #[allow(unused_variables)] gcp_project_id: Option<String>,
) -> Result<TracingGuard, Box<dyn std::error::Error + Send + Sync>> {
    let env_filter = EnvFilter::builder()
        .with_default_directive(LevelFilter::INFO.into())
        .from_env_lossy();

    #[cfg(feature = "otel")]
    if enable_otel {
        use tracing_subscriber::layer::SubscriberExt;

        // Configure OTLP exporter
        let exporter = opentelemetry_otlp::SpanExporter::builder()
            .with_tonic()
            .with_endpoint(
                std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT")
                    .unwrap_or_else(|_| "http://localhost:4317".to_string()),
            )
            .build()?;

        // Build tracer provider with service name resource
        let resource = opentelemetry_sdk::Resource::builder()
            .with_service_name(service_name.to_string())
            .build();

        let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder()
            .with_batch_exporter(exporter)
            .with_resource(resource)
            .build();

        // Create tracer from provider
        let tracer = provider.tracer(service_name.to_string());

        // Create OpenTelemetry layer for trace export
        let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);

        // Create logging layer based on configuration
        if let Some(project_id) = gcp_project_id {
            // Use tracing-stackdriver for Cloud Logging with trace correlation
            let cloud_trace_config = tracing_stackdriver::CloudTraceConfiguration { project_id };
            let stackdriver_layer =
                tracing_stackdriver::layer().with_cloud_trace(cloud_trace_config);

            let subscriber = tracing_subscriber::registry()
                .with(env_filter)
                .with(otel_layer)
                .with(stackdriver_layer);

            tracing::subscriber::set_global_default(subscriber)
                .expect("Failed to set global subscriber");
        } else if log_json {
            // JSON logging without Cloud Trace correlation
            let subscriber = tracing_subscriber::registry()
                .with(env_filter)
                .with(otel_layer)
                .with(tracing_subscriber::fmt::layer().json());

            tracing::subscriber::set_global_default(subscriber)
                .expect("Failed to set global subscriber");
        } else {
            // Pretty logging for development
            let subscriber = tracing_subscriber::registry()
                .with(env_filter)
                .with(otel_layer)
                .with(tracing_subscriber::fmt::layer().with_target(true));

            tracing::subscriber::set_global_default(subscriber)
                .expect("Failed to set global subscriber");
        }

        return Ok(TracingGuard {
            _provider: Some(provider),
        });
    }

    // Fallback: no OpenTelemetry, just regular tracing
    #[cfg(not(feature = "otel"))]
    let _ = enable_otel; // Suppress unused warning

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

    Ok(TracingGuard {
        #[cfg(feature = "otel")]
        _provider: None,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_init_without_otel() {
        // This should work without the otel feature
        // Can't actually call init_tracing in tests as it sets a global subscriber
        // Just verify the module compiles
    }
}
