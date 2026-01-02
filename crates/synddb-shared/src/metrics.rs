//! Shared metrics utilities for `SyndDB` services.
//!
//! This module provides common patterns for Prometheus metrics collection
//! across all `SyndDB` services. It includes:
//!
//! - Standard histogram buckets for common use cases
//! - A metrics handler for Axum HTTP APIs
//! - Helper macros and utilities
//!
//! # Google Cloud Integration
//!
//! These metrics are designed for scraping by Google Cloud Monitoring's
//! Prometheus endpoint. Services expose metrics at `/metrics` in the
//! standard Prometheus text format.
//!
//! # Usage
//!
//! Each service defines its own metrics in a dedicated module and uses
//! the shared utilities from here:
//!
//! ```rust,ignore
//! use synddb_shared::metrics::{LATENCY_BUCKETS_MS, metrics_handler};
//! use prometheus::{Histogram, HistogramOpts, register_histogram};
//!
//! let latency = register_histogram!(
//!     "my_operation_duration_seconds",
//!     "Duration of my operation",
//!     LATENCY_BUCKETS_SECONDS.to_vec()
//! ).unwrap();
//! ```

use axum::{http::StatusCode, response::IntoResponse};
use prometheus::{Encoder, TextEncoder};

/// Standard histogram buckets for latency measurements in seconds.
///
/// Covers a range from 1ms to 60s, suitable for most network operations.
/// Buckets: 1ms, 5ms, 10ms, 25ms, 50ms, 100ms, 250ms, 500ms, 1s, 2.5s, 5s, 10s, 30s, 60s
pub const LATENCY_BUCKETS_SECONDS: [f64; 14] = [
    0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0,
];

/// Standard histogram buckets for small latency measurements in seconds.
///
/// Covers a range from 100µs to 1s, suitable for fast operations like
/// database queries or in-memory processing.
/// Buckets: 100µs, 250µs, 500µs, 1ms, 2.5ms, 5ms, 10ms, 25ms, 50ms, 100ms, 250ms, 500ms, 1s
pub const LATENCY_BUCKETS_FAST: [f64; 13] = [
    0.0001, 0.00025, 0.0005, 0.001, 0.0025, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0,
];

/// Standard histogram buckets for byte sizes.
///
/// Covers a range from 100 bytes to 100MB, suitable for message and batch sizes.
/// Buckets: 100B, 1KB, 10KB, 100KB, 1MB, 10MB, 100MB
pub const SIZE_BUCKETS_BYTES: [f64; 7] = [
    100.0,
    1_024.0,
    10_240.0,
    102_400.0,
    1_048_576.0,
    10_485_760.0,
    104_857_600.0,
];

/// Axum handler that returns Prometheus metrics in text format.
///
/// Add this to your router:
/// ```rust,ignore
/// use synddb_shared::metrics::metrics_handler;
/// Router::new().route("/metrics", get(metrics_handler))
/// ```
pub async fn metrics_handler() -> impl IntoResponse {
    let encoder = TextEncoder::new();
    let metric_families = prometheus::gather();

    let mut buffer = Vec::new();
    match encoder.encode(&metric_families, &mut buffer) {
        Ok(()) => (
            StatusCode::OK,
            [(
                axum::http::header::CONTENT_TYPE,
                encoder.format_type().to_string(),
            )],
            buffer,
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to encode metrics: {e}"),
        )
            .into_response(),
    }
}

/// A guard that records the duration of an operation when dropped.
///
/// Useful for timing operations that may return early or have complex
/// control flow:
///
/// ```rust,ignore
/// use synddb_shared::metrics::HistogramTimer;
///
/// fn do_work(histogram: &Histogram) -> Result<()> {
///     let _timer = HistogramTimer::new(histogram);
///     // ... work happens here ...
///     // Timer records duration when dropped
/// }
/// ```
#[derive(Debug)]
pub struct HistogramTimer<'a> {
    histogram: &'a prometheus::Histogram,
    start: std::time::Instant,
}

impl<'a> HistogramTimer<'a> {
    /// Create a new timer that will record to the given histogram.
    pub fn new(histogram: &'a prometheus::Histogram) -> Self {
        Self {
            histogram,
            start: std::time::Instant::now(),
        }
    }
}

impl Drop for HistogramTimer<'_> {
    fn drop(&mut self) {
        let duration = self.start.elapsed();
        self.histogram.observe(duration.as_secs_f64());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prometheus::{register_histogram, Histogram};

    #[test]
    fn test_histogram_timer() {
        let histogram: Histogram =
            register_histogram!("test_timer_duration_seconds", "Test timer").unwrap();

        {
            let _timer = HistogramTimer::new(&histogram);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        // Should have recorded one observation
        assert_eq!(histogram.get_sample_count(), 1);
        // Should be at least 10ms
        assert!(histogram.get_sample_sum() >= 0.01);
    }

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
