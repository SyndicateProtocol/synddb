//! Metrics for the validator
//!
//! TODO: Implement Prometheus metrics endpoint. Planned metrics:
//!
//! - `synddb_validator_messages_synced_total` - Counter of synced messages
//! - `synddb_validator_last_sequence` - Gauge of last synced sequence number
//! - `synddb_validator_sync_lag_seconds` - Gauge of time since last sync
//! - `synddb_validator_changeset_apply_duration_seconds` - Histogram of apply times
//! - `synddb_validator_signature_verify_duration_seconds` - Histogram of verify times
//! - `synddb_validator_gaps_detected_total` - Counter of detected gaps
//! - `synddb_validator_withdrawals_signed_total` - Counter of signed withdrawals
