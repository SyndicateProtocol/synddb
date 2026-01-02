//! Prometheus metrics for the validator service.
//!
//! Exposes metrics at `/metrics` in Prometheus text format for scraping
//! by Google Cloud Monitoring.
//!
//! # Metrics
//!
//! ## Counters
//! - `synddb_validator_messages_synced_total` - Total messages synced (by type)
//! - `synddb_validator_gaps_detected_total` - Total sequence gaps detected
//! - `synddb_validator_withdrawals_processed_total` - Total withdrawals processed
//! - `synddb_validator_errors_total` - Total errors (by type)
//!
//! ## Histograms
//! - `synddb_validator_sync_duration_seconds` - Time to sync a single message
//! - `synddb_validator_apply_duration_seconds` - Time to apply a changeset
//! - `synddb_validator_verify_duration_seconds` - Time to verify a signature
//! - `synddb_validator_fetch_duration_seconds` - Time to fetch from storage
//!
//! ## Gauges
//! - `synddb_validator_last_sequence` - Last synced sequence number
//! - `synddb_validator_sync_lag_seconds` - Time since last sync
//! - `synddb_validator_pending_changesets` - Changesets awaiting verification

use prometheus::{
    register_counter_vec, register_gauge, register_histogram, register_histogram_vec, CounterVec,
    Gauge, Histogram, HistogramVec,
};
use std::sync::LazyLock;
use synddb_shared::metrics::{LATENCY_BUCKETS_FAST, LATENCY_BUCKETS_SECONDS};

// =============================================================================
// Counters
// =============================================================================

/// Total messages synced, labeled by message type
pub static MESSAGES_SYNCED_TOTAL: LazyLock<CounterVec> = LazyLock::new(|| {
    register_counter_vec!(
        "synddb_validator_messages_synced_total",
        "Total messages synced",
        &["type"]
    )
    .expect("Failed to register MESSAGES_SYNCED_TOTAL")
});

/// Total sequence gaps detected
pub static GAPS_DETECTED_TOTAL: LazyLock<Gauge> = LazyLock::new(|| {
    register_gauge!(
        "synddb_validator_gaps_detected_total",
        "Total sequence gaps detected"
    )
    .expect("Failed to register GAPS_DETECTED_TOTAL")
});

/// Total withdrawals processed
pub static WITHDRAWALS_PROCESSED_TOTAL: LazyLock<Gauge> = LazyLock::new(|| {
    register_gauge!(
        "synddb_validator_withdrawals_processed_total",
        "Total withdrawals processed"
    )
    .expect("Failed to register WITHDRAWALS_PROCESSED_TOTAL")
});

/// Total errors, labeled by error type
pub static ERRORS_TOTAL: LazyLock<CounterVec> = LazyLock::new(|| {
    register_counter_vec!(
        "synddb_validator_errors_total",
        "Total errors encountered",
        &["type"]
    )
    .expect("Failed to register ERRORS_TOTAL")
});

/// Total batches synced (for batch sync mode)
pub static BATCHES_SYNCED_TOTAL: LazyLock<Gauge> = LazyLock::new(|| {
    register_gauge!(
        "synddb_validator_batches_synced_total",
        "Total batches synced"
    )
    .expect("Failed to register BATCHES_SYNCED_TOTAL")
});

// =============================================================================
// Histograms
// =============================================================================

/// Time to sync a single message (fetch + verify + apply)
pub static SYNC_DURATION: LazyLock<Histogram> = LazyLock::new(|| {
    register_histogram!(
        "synddb_validator_sync_duration_seconds",
        "Time to sync a single message",
        LATENCY_BUCKETS_SECONDS.to_vec()
    )
    .expect("Failed to register SYNC_DURATION")
});

/// Time to apply a changeset to the database
pub static APPLY_DURATION: LazyLock<HistogramVec> = LazyLock::new(|| {
    register_histogram_vec!(
        "synddb_validator_apply_duration_seconds",
        "Time to apply a message",
        &["type"],
        LATENCY_BUCKETS_FAST.to_vec()
    )
    .expect("Failed to register APPLY_DURATION")
});

/// Time to verify a signature
pub static VERIFY_DURATION: LazyLock<Histogram> = LazyLock::new(|| {
    register_histogram!(
        "synddb_validator_verify_duration_seconds",
        "Time to verify a signature",
        LATENCY_BUCKETS_FAST.to_vec()
    )
    .expect("Failed to register VERIFY_DURATION")
});

/// Time to fetch from storage
pub static FETCH_DURATION: LazyLock<Histogram> = LazyLock::new(|| {
    register_histogram!(
        "synddb_validator_fetch_duration_seconds",
        "Time to fetch from storage",
        LATENCY_BUCKETS_SECONDS.to_vec()
    )
    .expect("Failed to register FETCH_DURATION")
});

/// Time to fetch a batch from storage
pub static BATCH_FETCH_DURATION: LazyLock<Histogram> = LazyLock::new(|| {
    register_histogram!(
        "synddb_validator_batch_fetch_duration_seconds",
        "Time to fetch a batch from storage",
        LATENCY_BUCKETS_SECONDS.to_vec()
    )
    .expect("Failed to register BATCH_FETCH_DURATION")
});

// =============================================================================
// Gauges
// =============================================================================

/// Last synced sequence number
pub static LAST_SEQUENCE: LazyLock<Gauge> = LazyLock::new(|| {
    register_gauge!(
        "synddb_validator_last_sequence",
        "Last synced sequence number"
    )
    .expect("Failed to register LAST_SEQUENCE")
});

/// Time since last successful sync (seconds)
pub static SYNC_LAG_SECONDS: LazyLock<Gauge> = LazyLock::new(|| {
    register_gauge!(
        "synddb_validator_sync_lag_seconds",
        "Time since last successful sync"
    )
    .expect("Failed to register SYNC_LAG_SECONDS")
});

/// Number of pending changesets awaiting verification
pub static PENDING_CHANGESETS: LazyLock<Gauge> = LazyLock::new(|| {
    register_gauge!(
        "synddb_validator_pending_changesets",
        "Changesets awaiting verification after snapshot"
    )
    .expect("Failed to register PENDING_CHANGESETS")
});

/// Whether the validator is currently syncing (1) or idle (0)
pub static SYNCING: LazyLock<Gauge> = LazyLock::new(|| {
    register_gauge!(
        "synddb_validator_syncing",
        "Whether the validator is syncing"
    )
    .expect("Failed to register SYNCING")
});

// =============================================================================
// Helper functions
// =============================================================================

/// Initialize all metrics (forces lazy initialization)
pub fn init() {
    let _ = &*MESSAGES_SYNCED_TOTAL;
    let _ = &*GAPS_DETECTED_TOTAL;
    let _ = &*WITHDRAWALS_PROCESSED_TOTAL;
    let _ = &*ERRORS_TOTAL;
    let _ = &*BATCHES_SYNCED_TOTAL;
    let _ = &*SYNC_DURATION;
    let _ = &*APPLY_DURATION;
    let _ = &*VERIFY_DURATION;
    let _ = &*FETCH_DURATION;
    let _ = &*BATCH_FETCH_DURATION;
    let _ = &*LAST_SEQUENCE;
    let _ = &*SYNC_LAG_SECONDS;
    let _ = &*PENDING_CHANGESETS;
    let _ = &*SYNCING;
}

/// Record a synced message
pub fn record_message_synced(message_type: &str, sequence: u64) {
    MESSAGES_SYNCED_TOTAL
        .with_label_values(&[message_type])
        .inc();
    LAST_SEQUENCE.set(sequence as f64);
    SYNC_LAG_SECONDS.set(0.0);
}

/// Record a gap detection
pub fn record_gap_detected() {
    GAPS_DETECTED_TOTAL.inc();
}

/// Record a withdrawal processed
pub fn record_withdrawal_processed() {
    WITHDRAWALS_PROCESSED_TOTAL.inc();
}

/// Record an error
pub fn record_error(error_type: &str) {
    ERRORS_TOTAL.with_label_values(&[error_type]).inc();
}

/// Record message apply duration
pub fn record_apply_duration(message_type: &str, duration_secs: f64) {
    APPLY_DURATION
        .with_label_values(&[message_type])
        .observe(duration_secs);
}

/// Record signature verification duration
pub fn record_verify_duration(duration_secs: f64) {
    VERIFY_DURATION.observe(duration_secs);
}

/// Record fetch duration
pub fn record_fetch_duration(duration_secs: f64) {
    FETCH_DURATION.observe(duration_secs);
}

/// Record batch fetch duration
pub fn record_batch_fetch_duration(duration_secs: f64) {
    BATCH_FETCH_DURATION.observe(duration_secs);
}

/// Record full sync duration (fetch + verify + apply)
pub fn record_sync_duration(duration_secs: f64) {
    SYNC_DURATION.observe(duration_secs);
}

/// Update pending changeset count
pub fn update_pending_changesets(count: u64) {
    PENDING_CHANGESETS.set(count as f64);
}

/// Update sync lag (time since last sync)
pub fn update_sync_lag(seconds: f64) {
    SYNC_LAG_SECONDS.set(seconds);
}

/// Set syncing state
pub fn set_syncing(syncing: bool) {
    SYNCING.set(if syncing { 1.0 } else { 0.0 });
}

/// Record a batch synced
pub fn record_batch_synced() {
    BATCHES_SYNCED_TOTAL.inc();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_init() {
        init();
    }

    #[test]
    fn test_record_message() {
        init();
        record_message_synced("changeset", 42);
    }
}
