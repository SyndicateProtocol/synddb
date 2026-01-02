//! Shared metrics utilities for `SyndDB` services.
//!
//! This module provides common patterns for metrics collection using the
//! `metrics` crate facade with a Prometheus exporter. This design allows:
//!
//! - Simple, ergonomic API: `counter!("name").increment(1)`
//! - Backend flexibility: can add other exporters without changing code
//! - Ecosystem consistency: same pattern as `tracing` for logs
//!
//! # Google Cloud Integration
//!
//! Metrics are exported in Prometheus text format at `/metrics` for scraping
//! by Google Cloud Managed Service for Prometheus.
//!
//! # Usage
//!
//! ```rust,ignore
//! use metrics::{counter, gauge, histogram};
//!
//! // Counters - monotonically increasing
//! counter!("requests_total", "endpoint" => "/api").increment(1);
//!
//! // Gauges - can go up or down
//! gauge!("connections_active").set(42.0);
//!
//! // Histograms - for latency/size distributions
//! histogram!("request_duration_seconds").record(0.025);
//! ```
//!
//! # Initialization
//!
//! Call [`init_metrics`] once at startup before recording any metrics:
//!
//! ```rust,ignore
//! use synddb_shared::metrics::init_metrics;
//!
//! #[tokio::main]
//! async fn main() {
//!     let handle = init_metrics();
//!     // ... your app ...
//! }
//! ```

use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use std::sync::OnceLock;

/// Global handle to the Prometheus recorder, initialized once.
static METRICS_HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();

/// Standard histogram buckets for latency measurements in seconds.
///
/// Covers a range from 1ms to 60s, suitable for most network operations.
pub const LATENCY_BUCKETS_SECONDS: &[f64] = &[
    0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0,
];

/// Standard histogram buckets for fast operations in seconds.
///
/// Covers a range from 100µs to 1s, suitable for database queries
/// or in-memory processing.
pub const LATENCY_BUCKETS_FAST: &[f64] = &[
    0.0001, 0.00025, 0.0005, 0.001, 0.0025, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0,
];

/// Standard histogram buckets for byte sizes.
///
/// Covers a range from 100 bytes to 100MB.
pub const SIZE_BUCKETS_BYTES: &[f64] = &[
    100.0,
    1_024.0,
    10_240.0,
    102_400.0,
    1_048_576.0,
    10_485_760.0,
    104_857_600.0,
];

/// Initialize the Prometheus metrics exporter.
///
/// Returns a handle that can be used to render metrics. This function is
/// idempotent - calling it multiple times returns the same handle.
///
/// # Panics
///
/// Panics if the initial installation of the recorder fails.
pub fn init_metrics() -> PrometheusHandle {
    METRICS_HANDLE
        .get_or_init(|| {
            PrometheusBuilder::new()
                .set_buckets_for_metric(
                    metrics_exporter_prometheus::Matcher::Suffix("_duration_seconds".to_string()),
                    LATENCY_BUCKETS_SECONDS,
                )
                .expect("Failed to set duration buckets")
                .set_buckets_for_metric(
                    metrics_exporter_prometheus::Matcher::Suffix("_size_bytes".to_string()),
                    SIZE_BUCKETS_BYTES,
                )
                .expect("Failed to set size buckets")
                .install_recorder()
                .expect("Failed to install Prometheus recorder")
        })
        .clone()
}

/// Create an Axum handler that renders Prometheus metrics.
///
/// Use with the handle returned from [`init_metrics`]:
///
/// ```rust,ignore
/// use axum::{Router, routing::get};
/// use synddb_shared::metrics::{init_metrics, metrics_endpoint};
///
/// let handle = init_metrics();
/// let app = Router::new()
///     .route("/metrics", get(metrics_endpoint(handle)));
/// ```
pub fn metrics_endpoint(
    handle: PrometheusHandle,
) -> impl Fn() -> std::future::Ready<String> + Clone + Send + 'static {
    move || std::future::ready(handle.render())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_latency_buckets() {
        // Verify bucket ordering
        for window in LATENCY_BUCKETS_SECONDS.windows(2) {
            assert!(window[0] < window[1], "Buckets must be ascending");
        }
        for window in LATENCY_BUCKETS_FAST.windows(2) {
            assert!(window[0] < window[1], "Buckets must be ascending");
        }
        for window in SIZE_BUCKETS_BYTES.windows(2) {
            assert!(window[0] < window[1], "Buckets must be ascending");
        }
    }
}
