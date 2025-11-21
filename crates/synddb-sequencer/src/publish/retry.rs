//! Retry logic with exponential backoff

use anyhow::Result;
use std::time::Duration;
use tokio::time::sleep;

#[derive(Debug)]
pub struct RetryPolicy {
    max_retries: usize,
    initial_backoff: Duration,
    max_backoff: Duration,
    multiplier: f64,
}

impl RetryPolicy {
    pub const fn new(max_retries: usize) -> Self {
        Self {
            max_retries,
            initial_backoff: Duration::from_millis(100),
            max_backoff: Duration::from_secs(30),
            multiplier: 2.0,
        }
    }

    /// Execute function with retry logic
    pub async fn execute<F, T, Fut>(&self, mut f: F) -> Result<T>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        let mut backoff = self.initial_backoff;

        for attempt in 0..=self.max_retries {
            match f().await {
                Ok(result) => return Ok(result),
                Err(e) if attempt < self.max_retries => {
                    tracing::warn!(
                        "Attempt {} failed: {}, retrying in {:?}",
                        attempt,
                        e,
                        backoff
                    );
                    sleep(backoff).await;
                    backoff = std::cmp::min(
                        Duration::from_secs_f64(backoff.as_secs_f64() * self.multiplier),
                        self.max_backoff,
                    );
                }
                Err(e) => return Err(e),
            }
        }

        unreachable!()
    }
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self::new(3)
    }
}
