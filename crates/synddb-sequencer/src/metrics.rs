//! Prometheus metrics for the sequencer service.
//!
//! Uses the `metrics` crate facade which exports to Prometheus format for scraping
//! by Google Cloud Managed Service for Prometheus.
//!
//! # Metrics
//!
//! ## Counters
//! - `synddb_sequencer_messages_total` - Total messages sequenced (by type)
//! - `synddb_sequencer_batches_total` - Total batches published
//! - `synddb_sequencer_bytes_published_total` - Total bytes published (compressed)
//! - `synddb_sequencer_bytes_uncompressed_total` - Total uncompressed bytes
//! - `synddb_sequencer_errors_total` - Total errors (by type)
//!
//! ## Histograms
//! - `synddb_sequencer_sequence_duration_seconds` - Time to sequence a message
//! - `synddb_sequencer_batch_publish_duration_seconds` - Time to publish a batch
//! - `synddb_sequencer_message_size_bytes` - Size of sequenced messages
//! - `synddb_sequencer_batch_size_bytes` - Size of published batches
//!
//! ## Gauges
//! - `synddb_sequencer_current_sequence` - Current sequence number
//! - `synddb_sequencer_pending_messages` - Messages waiting to be batched
//! - `synddb_sequencer_pending_bytes` - Bytes waiting to be batched
//! - `synddb_sequencer_compression_ratio` - Current compression ratio

use metrics::{counter, gauge, histogram};
use std::sync::atomic::{AtomicU64, Ordering};

// Track totals for compression ratio calculation
static BYTES_PUBLISHED: AtomicU64 = AtomicU64::new(0);
static BYTES_UNCOMPRESSED: AtomicU64 = AtomicU64::new(0);

// =============================================================================
// Helper functions
// =============================================================================

/// Record a sequenced message
pub fn record_message_sequenced(message_type: &str, size_bytes: usize, duration_secs: f64) {
    counter!("synddb_sequencer_messages_total", "type" => message_type.to_string()).increment(1);
    histogram!("synddb_sequencer_message_size_bytes", "type" => message_type.to_string())
        .record(size_bytes as f64);
    histogram!("synddb_sequencer_sequence_duration_seconds", "type" => message_type.to_string())
        .record(duration_secs);
}

/// Record a published batch
pub fn record_batch_published(
    compressed_bytes: usize,
    uncompressed_bytes: usize,
    duration_secs: f64,
) {
    counter!("synddb_sequencer_batches_total").increment(1);
    counter!("synddb_sequencer_bytes_published_total").increment(compressed_bytes as u64);
    counter!("synddb_sequencer_bytes_uncompressed_total").increment(uncompressed_bytes as u64);
    histogram!("synddb_sequencer_batch_size_bytes").record(compressed_bytes as f64);
    histogram!("synddb_sequencer_batch_publish_duration_seconds").record(duration_secs);

    // Update compression ratio
    let total_compressed = BYTES_PUBLISHED.fetch_add(compressed_bytes as u64, Ordering::Relaxed)
        + compressed_bytes as u64;
    let total_uncompressed = BYTES_UNCOMPRESSED
        .fetch_add(uncompressed_bytes as u64, Ordering::Relaxed)
        + uncompressed_bytes as u64;
    if total_compressed > 0 {
        gauge!("synddb_sequencer_compression_ratio")
            .set(total_uncompressed as f64 / total_compressed as f64);
    }
}

/// Update pending batch metrics
pub fn update_pending(messages: usize, bytes: usize) {
    gauge!("synddb_sequencer_pending_messages").set(messages as f64);
    gauge!("synddb_sequencer_pending_bytes").set(bytes as f64);
}

/// Update current sequence gauge
pub fn update_current_sequence(sequence: u64) {
    gauge!("synddb_sequencer_current_sequence").set(sequence as f64);
}

/// Record an error
pub fn record_error(error_type: &str) {
    counter!("synddb_sequencer_errors_total", "type" => error_type.to_string()).increment(1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_message() {
        // These should not panic even without a recorder installed
        record_message_sequenced("changeset", 1024, 0.001);
        record_batch_published(512, 1024, 0.05);
        update_pending(10, 5000);
        update_current_sequence(42);
        record_error("test_error");
    }
}
