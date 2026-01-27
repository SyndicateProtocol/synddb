//! Prometheus metrics for the validator service.
//!
//! Uses the `metrics` crate facade which exports to Prometheus format for scraping
//! by Google Cloud Managed Service for Prometheus.
//!
//! # Metrics
//!
//! ## Counters
//! - `synddb_validator_messages_synced_total` - Total messages synced (by type)
//! - `synddb_validator_gaps_detected_total` - Total sequence gaps detected
//! - `synddb_validator_withdrawals_processed_total` - Total withdrawals processed
//! - `synddb_validator_errors_total` - Total errors (by type)
//! - `synddb_validator_batches_synced_total` - Total batches synced
//!
//! ## Histograms
//! - `synddb_validator_sync_duration_seconds` - Time to sync a single message
//! - `synddb_validator_apply_duration_seconds` - Time to apply a changeset
//! - `synddb_validator_verify_duration_seconds` - Time to verify a signature
//! - `synddb_validator_fetch_duration_seconds` - Time to fetch from storage
//! - `synddb_validator_batch_fetch_duration_seconds` - Time to fetch a batch
//!
//! ## Gauges
//! - `synddb_validator_last_sequence` - Last synced sequence number
//! - `synddb_validator_sync_lag_seconds` - Time since last sync
//! - `synddb_validator_pending_changesets` - Changesets awaiting verification
//! - `synddb_validator_syncing` - Whether the validator is syncing

use metrics::{counter, gauge, histogram};

// =============================================================================
// Helper functions
// =============================================================================

/// Record a synced message
pub fn record_message_synced(message_type: &str, sequence: u64) {
    counter!("synddb_validator_messages_synced_total", "type" => message_type.to_string())
        .increment(1);
    gauge!("synddb_validator_last_sequence").set(sequence as f64);
    gauge!("synddb_validator_sync_lag_seconds").set(0.0);
}

/// Record a gap detection
pub fn record_gap_detected() {
    counter!("synddb_validator_gaps_detected_total").increment(1);
}

/// Record a withdrawal processed
pub fn record_withdrawal_processed() {
    counter!("synddb_validator_withdrawals_processed_total").increment(1);
}

/// Record an error
pub fn record_error(error_type: &str) {
    counter!("synddb_validator_errors_total", "type" => error_type.to_string()).increment(1);
}

/// Record message apply duration
pub fn record_apply_duration(message_type: &str, duration_secs: f64) {
    histogram!("synddb_validator_apply_duration_seconds", "type" => message_type.to_string())
        .record(duration_secs);
}

/// Record signature verification duration
pub fn record_verify_duration(duration_secs: f64) {
    histogram!("synddb_validator_verify_duration_seconds").record(duration_secs);
}

/// Record fetch duration
pub fn record_fetch_duration(duration_secs: f64) {
    histogram!("synddb_validator_fetch_duration_seconds").record(duration_secs);
}

/// Record batch fetch duration
pub fn record_batch_fetch_duration(duration_secs: f64) {
    histogram!("synddb_validator_batch_fetch_duration_seconds").record(duration_secs);
}

/// Record full sync duration (fetch + verify + apply)
pub fn record_sync_duration(duration_secs: f64) {
    histogram!("synddb_validator_sync_duration_seconds").record(duration_secs);
}

/// Update pending changeset count
pub fn update_pending_changesets(count: u64) {
    gauge!("synddb_validator_pending_changesets").set(count as f64);
}

/// Update sync lag (time since last sync)
pub fn update_sync_lag(seconds: f64) {
    gauge!("synddb_validator_sync_lag_seconds").set(seconds);
}

/// Set syncing state
pub fn set_syncing(syncing: bool) {
    gauge!("synddb_validator_syncing").set(if syncing { 1.0 } else { 0.0 });
}

/// Record a batch synced
pub fn record_batch_synced() {
    counter!("synddb_validator_batches_synced_total").increment(1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_message() {
        // These should not panic even without a recorder installed
        record_message_synced("changeset", 42);
        record_gap_detected();
        record_withdrawal_processed();
        record_error("test_error");
        record_apply_duration("changeset", 0.001);
        record_verify_duration(0.0005);
        record_fetch_duration(0.05);
        record_batch_fetch_duration(0.1);
        record_sync_duration(0.15);
        update_pending_changesets(10);
        update_sync_lag(5.0);
        set_syncing(true);
        record_batch_synced();
    }
}
