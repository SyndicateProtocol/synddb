//! Prometheus metrics for the sequencer service.
//!
//! Exposes metrics at `/metrics` in Prometheus text format for scraping
//! by Google Cloud Monitoring.
//!
//! # Metrics
//!
//! ## Counters
//! - `synddb_sequencer_messages_total` - Total messages sequenced (by type)
//! - `synddb_sequencer_batches_total` - Total batches published
//! - `synddb_sequencer_bytes_total` - Total bytes published (compressed)
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

use prometheus::{
    register_counter_vec, register_gauge, register_histogram, register_histogram_vec, CounterVec,
    Gauge, Histogram, HistogramVec,
};
use std::sync::LazyLock;
use synddb_shared::metrics::{LATENCY_BUCKETS_FAST, LATENCY_BUCKETS_SECONDS, SIZE_BUCKETS_BYTES};

// =============================================================================
// Counters
// =============================================================================

/// Total messages sequenced, labeled by message type
pub static MESSAGES_TOTAL: LazyLock<CounterVec> = LazyLock::new(|| {
    register_counter_vec!(
        "synddb_sequencer_messages_total",
        "Total messages sequenced",
        &["type"]
    )
    .expect("Failed to register MESSAGES_TOTAL")
});

/// Total batches published
pub static BATCHES_TOTAL: LazyLock<Gauge> = LazyLock::new(|| {
    register_gauge!("synddb_sequencer_batches_total", "Total batches published")
        .expect("Failed to register BATCHES_TOTAL")
});

/// Total compressed bytes published
pub static BYTES_PUBLISHED_TOTAL: LazyLock<Gauge> = LazyLock::new(|| {
    register_gauge!(
        "synddb_sequencer_bytes_published_total",
        "Total compressed bytes published"
    )
    .expect("Failed to register BYTES_PUBLISHED_TOTAL")
});

/// Total uncompressed bytes (for compression ratio)
pub static BYTES_UNCOMPRESSED_TOTAL: LazyLock<Gauge> = LazyLock::new(|| {
    register_gauge!(
        "synddb_sequencer_bytes_uncompressed_total",
        "Total uncompressed bytes before compression"
    )
    .expect("Failed to register BYTES_UNCOMPRESSED_TOTAL")
});

/// Total errors, labeled by error type
pub static ERRORS_TOTAL: LazyLock<CounterVec> = LazyLock::new(|| {
    register_counter_vec!(
        "synddb_sequencer_errors_total",
        "Total errors encountered",
        &["type"]
    )
    .expect("Failed to register ERRORS_TOTAL")
});

// =============================================================================
// Histograms
// =============================================================================

/// Time to sequence a message (signing + CBOR encoding)
pub static SEQUENCE_DURATION: LazyLock<HistogramVec> = LazyLock::new(|| {
    register_histogram_vec!(
        "synddb_sequencer_sequence_duration_seconds",
        "Time to sequence a message",
        &["type"],
        LATENCY_BUCKETS_FAST.to_vec()
    )
    .expect("Failed to register SEQUENCE_DURATION")
});

/// Time to publish a batch to storage
pub static BATCH_PUBLISH_DURATION: LazyLock<Histogram> = LazyLock::new(|| {
    register_histogram!(
        "synddb_sequencer_batch_publish_duration_seconds",
        "Time to publish a batch to storage",
        LATENCY_BUCKETS_SECONDS.to_vec()
    )
    .expect("Failed to register BATCH_PUBLISH_DURATION")
});

/// Size of individual sequenced messages (compressed payload)
pub static MESSAGE_SIZE: LazyLock<HistogramVec> = LazyLock::new(|| {
    register_histogram_vec!(
        "synddb_sequencer_message_size_bytes",
        "Size of sequenced messages in bytes",
        &["type"],
        SIZE_BUCKETS_BYTES.to_vec()
    )
    .expect("Failed to register MESSAGE_SIZE")
});

/// Size of published batches (compressed)
pub static BATCH_SIZE: LazyLock<Histogram> = LazyLock::new(|| {
    register_histogram!(
        "synddb_sequencer_batch_size_bytes",
        "Size of published batches in bytes",
        SIZE_BUCKETS_BYTES.to_vec()
    )
    .expect("Failed to register BATCH_SIZE")
});

// =============================================================================
// Gauges
// =============================================================================

/// Current sequence number (next to be assigned)
pub static CURRENT_SEQUENCE: LazyLock<Gauge> = LazyLock::new(|| {
    register_gauge!(
        "synddb_sequencer_current_sequence",
        "Current sequence number (next to be assigned)"
    )
    .expect("Failed to register CURRENT_SEQUENCE")
});

/// Number of messages waiting to be batched
pub static PENDING_MESSAGES: LazyLock<Gauge> = LazyLock::new(|| {
    register_gauge!(
        "synddb_sequencer_pending_messages",
        "Messages waiting to be batched"
    )
    .expect("Failed to register PENDING_MESSAGES")
});

/// Bytes waiting to be batched
pub static PENDING_BYTES: LazyLock<Gauge> = LazyLock::new(|| {
    register_gauge!(
        "synddb_sequencer_pending_bytes",
        "Bytes waiting to be batched"
    )
    .expect("Failed to register PENDING_BYTES")
});

/// Compression ratio (uncompressed / compressed)
pub static COMPRESSION_RATIO: LazyLock<Gauge> = LazyLock::new(|| {
    register_gauge!(
        "synddb_sequencer_compression_ratio",
        "Current compression ratio (uncompressed / compressed)"
    )
    .expect("Failed to register COMPRESSION_RATIO")
});

// =============================================================================
// Helper functions
// =============================================================================

/// Initialize all metrics (forces lazy initialization)
pub fn init() {
    // Touch each metric to ensure it's registered
    let _ = &*MESSAGES_TOTAL;
    let _ = &*BATCHES_TOTAL;
    let _ = &*BYTES_PUBLISHED_TOTAL;
    let _ = &*BYTES_UNCOMPRESSED_TOTAL;
    let _ = &*ERRORS_TOTAL;
    let _ = &*SEQUENCE_DURATION;
    let _ = &*BATCH_PUBLISH_DURATION;
    let _ = &*MESSAGE_SIZE;
    let _ = &*BATCH_SIZE;
    let _ = &*CURRENT_SEQUENCE;
    let _ = &*PENDING_MESSAGES;
    let _ = &*PENDING_BYTES;
    let _ = &*COMPRESSION_RATIO;
}

/// Record a sequenced message
pub fn record_message_sequenced(message_type: &str, size_bytes: usize, duration_secs: f64) {
    MESSAGES_TOTAL.with_label_values(&[message_type]).inc();
    MESSAGE_SIZE
        .with_label_values(&[message_type])
        .observe(size_bytes as f64);
    SEQUENCE_DURATION
        .with_label_values(&[message_type])
        .observe(duration_secs);
}

/// Record a published batch
pub fn record_batch_published(
    compressed_bytes: usize,
    uncompressed_bytes: usize,
    duration_secs: f64,
) {
    BATCHES_TOTAL.inc();
    BYTES_PUBLISHED_TOTAL.add(compressed_bytes as f64);
    BYTES_UNCOMPRESSED_TOTAL.add(uncompressed_bytes as f64);
    BATCH_SIZE.observe(compressed_bytes as f64);
    BATCH_PUBLISH_DURATION.observe(duration_secs);

    // Update compression ratio
    let total_compressed = BYTES_PUBLISHED_TOTAL.get();
    let total_uncompressed = BYTES_UNCOMPRESSED_TOTAL.get();
    if total_compressed > 0.0 {
        COMPRESSION_RATIO.set(total_uncompressed / total_compressed);
    }
}

/// Update pending batch metrics
pub fn update_pending(messages: usize, bytes: usize) {
    PENDING_MESSAGES.set(messages as f64);
    PENDING_BYTES.set(bytes as f64);
}

/// Update current sequence gauge
pub fn update_current_sequence(sequence: u64) {
    CURRENT_SEQUENCE.set(sequence as f64);
}

/// Record an error
pub fn record_error(error_type: &str) {
    ERRORS_TOTAL.with_label_values(&[error_type]).inc();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_init() {
        init();
        // Should not panic
    }

    #[test]
    fn test_record_message() {
        init();
        record_message_sequenced("changeset", 1024, 0.001);
        // Verify counter incremented (would need to check via prometheus gather)
    }
}
