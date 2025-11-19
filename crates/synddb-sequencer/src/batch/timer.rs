//! Time-based triggers for batching

use std::time::Duration;
use tokio::time::{interval, Interval};

pub struct BatchTimer {
    flush_interval: Interval,
    snapshot_interval: Interval,
}

impl BatchTimer {
    pub fn new(flush_duration: Duration, snapshot_duration: Duration) -> Self {
        Self {
            flush_interval: interval(flush_duration),
            snapshot_interval: interval(snapshot_duration),
        }
    }

    /// Wait for next flush trigger
    pub async fn next_flush(&mut self) {
        self.flush_interval.tick().await;
    }

    /// Wait for next snapshot trigger
    pub async fn next_snapshot(&mut self) {
        self.snapshot_interval.tick().await;
    }
}
