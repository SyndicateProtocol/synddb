//! Common retry logic for sending data to the sequencer

use std::time::Duration;
use tracing::warn;

/// Retry an async operation with exponential backoff
///
/// # Arguments
/// * `max_retries` - Maximum number of retry attempts
/// * `operation` - Async function to retry
///
/// # Returns
/// `Ok(())` if any attempt succeeds, `Err(last_error)` if all attempts fail
pub async fn retry_with_backoff<F, Fut, E>(max_retries: usize, mut operation: F) -> Result<(), E>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<(), E>>,
    E: std::fmt::Display,
{
    for attempt in 0..max_retries {
        match operation().await {
            Ok(()) => return Ok(()),
            Err(e) => {
                if attempt + 1 < max_retries {
                    let backoff = Duration::from_secs(1 << attempt);
                    warn!(
                        "Attempt {} failed: {}. Sleeping for {:?}",
                        attempt + 1,
                        e,
                        backoff
                    );
                    tokio::time::sleep(backoff).await;
                } else {
                    return Err(e);
                }
            }
        }
    }
    unreachable!()
}
