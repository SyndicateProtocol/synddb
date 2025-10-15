//! Performance metrics collection for SyndDB
//!
//! This module provides lightweight performance tracking for database operations,
//! including latency histograms, throughput counters, and operation tracking.

use parking_lot::RwLock;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Performance metrics collector
pub struct MetricsCollector {
    /// Operation latency tracker
    latencies: RwLock<LatencyTracker>,
    /// Throughput counter
    throughput: RwLock<ThroughputCounter>,
    /// Error counter
    errors: RwLock<ErrorCounter>,
}

/// Tracks operation latencies with histogram
struct LatencyTracker {
    /// Recent latencies (circular buffer)
    samples: VecDeque<Duration>,
    /// Maximum samples to keep
    max_samples: usize,
    /// Sorted samples for percentile calculation (updated lazily)
    sorted_cache: Option<Vec<Duration>>,
}

/// Tracks operations per second
struct ThroughputCounter {
    /// Total operations
    total_ops: u64,
    /// Operations in current window
    window_ops: u64,
    /// Window start time
    window_start: Instant,
    /// Window duration
    window_duration: Duration,
}

/// Tracks error counts by type
struct ErrorCounter {
    /// Total errors
    total_errors: u64,
    /// Errors in current window
    window_errors: u64,
    /// Window start time
    window_start: Instant,
}

impl MetricsCollector {
    /// Create a new metrics collector
    pub fn new() -> Self {
        Self {
            latencies: RwLock::new(LatencyTracker::new(10000)),
            throughput: RwLock::new(ThroughputCounter::new(Duration::from_secs(1))),
            errors: RwLock::new(ErrorCounter::new()),
        }
    }

    /// Record an operation latency
    pub fn record_latency(&self, duration: Duration) {
        let mut latencies = self.latencies.write();
        latencies.add(duration);

        let mut throughput = self.throughput.write();
        throughput.increment();
    }

    /// Record an error
    pub fn record_error(&self) {
        let mut errors = self.errors.write();
        errors.increment();
    }

    /// Get current metrics snapshot
    pub fn snapshot(&self) -> MetricsSnapshot {
        let mut latencies = self.latencies.write();
        let throughput = self.throughput.read();
        let errors = self.errors.read();

        MetricsSnapshot {
            total_operations: throughput.total_ops,
            ops_per_second: throughput.current_ops_per_second(),
            total_errors: errors.total_errors,
            error_rate: errors.current_error_rate(throughput.total_ops),
            avg_latency_us: latencies.mean().map(|d| d.as_micros() as f64),
            p50_latency_us: latencies.percentile(50.0).map(|d| d.as_micros() as u64),
            p95_latency_us: latencies.percentile(95.0).map(|d| d.as_micros() as u64),
            p99_latency_us: latencies.percentile(99.0).map(|d| d.as_micros() as u64),
            p999_latency_us: latencies.percentile(99.9).map(|d| d.as_micros() as u64),
            min_latency_us: latencies.min().map(|d| d.as_micros() as u64),
            max_latency_us: latencies.max().map(|d| d.as_micros() as u64),
        }
    }

    /// Reset all metrics
    pub fn reset(&self) {
        let mut latencies = self.latencies.write();
        latencies.clear();

        let mut throughput = self.throughput.write();
        throughput.reset();

        let mut errors = self.errors.write();
        errors.reset();
    }
}

impl Default for MetricsCollector {
    fn default() -> Self {
        Self::new()
    }
}

/// Snapshot of metrics at a point in time
#[derive(Debug, Clone)]
pub struct MetricsSnapshot {
    /// Total operations executed
    pub total_operations: u64,
    /// Operations per second (current window)
    pub ops_per_second: f64,
    /// Total errors encountered
    pub total_errors: u64,
    /// Error rate as percentage
    pub error_rate: f64,
    /// Average latency in microseconds
    pub avg_latency_us: Option<f64>,
    /// 50th percentile latency
    pub p50_latency_us: Option<u64>,
    /// 95th percentile latency
    pub p95_latency_us: Option<u64>,
    /// 99th percentile latency
    pub p99_latency_us: Option<u64>,
    /// 99.9th percentile latency
    pub p999_latency_us: Option<u64>,
    /// Minimum latency
    pub min_latency_us: Option<u64>,
    /// Maximum latency
    pub max_latency_us: Option<u64>,
}

impl MetricsSnapshot {
    /// Format as human-readable string
    pub fn format(&self) -> String {
        format!(
            "Operations: {} ({:.2} ops/s) | Errors: {} ({:.2}%) | \
             Latency: p50={:?}μs p99={:?}μs p99.9={:?}μs avg={:.2?}μs",
            self.total_operations,
            self.ops_per_second,
            self.total_errors,
            self.error_rate,
            self.p50_latency_us,
            self.p99_latency_us,
            self.p999_latency_us,
            self.avg_latency_us,
        )
    }
}

// ============================================================================
// Latency Tracker Implementation
// ============================================================================

impl LatencyTracker {
    fn new(max_samples: usize) -> Self {
        Self {
            samples: VecDeque::with_capacity(max_samples),
            max_samples,
            sorted_cache: None,
        }
    }

    fn add(&mut self, duration: Duration) {
        if self.samples.len() >= self.max_samples {
            self.samples.pop_front();
        }
        self.samples.push_back(duration);
        // Invalidate sorted cache
        self.sorted_cache = None;
    }

    fn clear(&mut self) {
        self.samples.clear();
        self.sorted_cache = None;
    }

    fn mean(&self) -> Option<Duration> {
        if self.samples.is_empty() {
            return None;
        }

        let sum: Duration = self.samples.iter().sum();
        Some(sum / self.samples.len() as u32)
    }

    fn percentile(&mut self, p: f64) -> Option<Duration> {
        if self.samples.is_empty() {
            return None;
        }

        // Update sorted cache if needed
        if self.sorted_cache.is_none() {
            let mut sorted: Vec<Duration> = self.samples.iter().copied().collect();
            sorted.sort();
            self.sorted_cache = Some(sorted);
        }

        let sorted = self.sorted_cache.as_ref().unwrap();
        let index = ((p / 100.0) * (sorted.len() as f64 - 1.0)).round() as usize;
        Some(sorted[index])
    }

    fn min(&self) -> Option<Duration> {
        self.samples.iter().min().copied()
    }

    fn max(&self) -> Option<Duration> {
        self.samples.iter().max().copied()
    }
}

// ============================================================================
// Throughput Counter Implementation
// ============================================================================

impl ThroughputCounter {
    fn new(window_duration: Duration) -> Self {
        Self {
            total_ops: 0,
            window_ops: 0,
            window_start: Instant::now(),
            window_duration,
        }
    }

    fn increment(&mut self) {
        self.total_ops += 1;
        self.window_ops += 1;

        // Check if window has elapsed
        let elapsed = self.window_start.elapsed();
        if elapsed >= self.window_duration {
            // Reset window
            self.window_ops = 0;
            self.window_start = Instant::now();
        }
    }

    fn current_ops_per_second(&self) -> f64 {
        let elapsed = self.window_start.elapsed();
        if elapsed.as_secs_f64() == 0.0 {
            return 0.0;
        }
        self.window_ops as f64 / elapsed.as_secs_f64()
    }

    fn reset(&mut self) {
        self.total_ops = 0;
        self.window_ops = 0;
        self.window_start = Instant::now();
    }
}

// ============================================================================
// Error Counter Implementation
// ============================================================================

impl ErrorCounter {
    fn new() -> Self {
        Self {
            total_errors: 0,
            window_errors: 0,
            window_start: Instant::now(),
        }
    }

    fn increment(&mut self) {
        self.total_errors += 1;
        self.window_errors += 1;
    }

    fn current_error_rate(&self, total_ops: u64) -> f64 {
        if total_ops == 0 {
            return 0.0;
        }
        (self.total_errors as f64 / total_ops as f64) * 100.0
    }

    fn reset(&mut self) {
        self.total_errors = 0;
        self.window_errors = 0;
        self.window_start = Instant::now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_collection() {
        let metrics = MetricsCollector::new();

        // Record some operations
        metrics.record_latency(Duration::from_micros(100));
        metrics.record_latency(Duration::from_micros(200));
        metrics.record_latency(Duration::from_micros(300));

        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.total_operations, 3);
        assert!(snapshot.avg_latency_us.is_some());
        assert!(snapshot.p50_latency_us.is_some());
    }

    #[test]
    fn test_latency_percentiles() {
        let metrics = MetricsCollector::new();

        // Record 100 operations with predictable latencies
        for i in 1..=100 {
            metrics.record_latency(Duration::from_micros(i * 10));
        }

        let snapshot = metrics.snapshot();

        // p50 should be around 500μs (50th of 100 values)
        assert!(snapshot.p50_latency_us.unwrap() >= 490);
        assert!(snapshot.p50_latency_us.unwrap() <= 510);

        // p99 should be around 990μs (99th of 100 values)
        assert!(snapshot.p99_latency_us.unwrap() >= 980);
        assert!(snapshot.p99_latency_us.unwrap() <= 1000);
    }

    #[test]
    fn test_throughput_calculation() {
        let metrics = MetricsCollector::new();

        // Record multiple operations
        for _ in 0..10 {
            metrics.record_latency(Duration::from_micros(100));
        }

        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.total_operations, 10);
        assert!(snapshot.ops_per_second > 0.0);
    }

    #[test]
    fn test_error_tracking() {
        let metrics = MetricsCollector::new();

        // Record some operations and errors
        for _ in 0..10 {
            metrics.record_latency(Duration::from_micros(100));
        }
        metrics.record_error();
        metrics.record_error();

        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.total_errors, 2);
        assert_eq!(snapshot.error_rate, 20.0); // 2/10 = 20%
    }

    #[test]
    fn test_metrics_reset() {
        let metrics = MetricsCollector::new();

        // Record some data
        metrics.record_latency(Duration::from_micros(100));
        metrics.record_error();

        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.total_operations, 1);
        assert_eq!(snapshot.total_errors, 1);

        // Reset
        metrics.reset();

        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.total_operations, 0);
        assert_eq!(snapshot.total_errors, 0);
    }

    #[test]
    fn test_latency_tracker_circular_buffer() {
        let mut tracker = LatencyTracker::new(3); // Max 3 samples

        tracker.add(Duration::from_micros(100));
        tracker.add(Duration::from_micros(200));
        tracker.add(Duration::from_micros(300));
        assert_eq!(tracker.samples.len(), 3);

        // Adding 4th should remove first
        tracker.add(Duration::from_micros(400));
        assert_eq!(tracker.samples.len(), 3);
        assert_eq!(tracker.min().unwrap(), Duration::from_micros(200));
        assert_eq!(tracker.max().unwrap(), Duration::from_micros(400));
    }

    #[test]
    fn test_snapshot_format() {
        let metrics = MetricsCollector::new();
        metrics.record_latency(Duration::from_micros(100));

        let snapshot = metrics.snapshot();
        let formatted = snapshot.format();

        assert!(formatted.contains("Operations:"));
        assert!(formatted.contains("ops/s"));
        assert!(formatted.contains("Latency:"));
    }
}
