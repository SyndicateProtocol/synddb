//! Generic message handler trait for processing blockchain events.
//!
//! This module defines the trait that applications implement to process
//! blockchain events in a custom way.

use alloy::{primitives::B256, rpc::types::Log};
use anyhow::Result;
use std::fmt::Debug;

/// Generic trait for processing blockchain events.
///
/// Applications implement this trait to define custom event handling logic.
/// The chain monitor will call `handle_event` for each event it receives,
/// allowing the application to process it in any way needed.
///
/// # Example
///
/// ```
/// use synddb_chain_monitor::MessageHandler;
/// use alloy::{rpc::types::Log, primitives::B256};
/// use anyhow::Result;
///
/// struct MyHandler {
///     // Your application state
/// }
///
/// #[async_trait::async_trait]
/// impl MessageHandler for MyHandler {
///     async fn handle_event(&self, log: &Log) -> Result<bool> {
///         // Process the event
///         println!("Received event from block {:?}", log.block_number);
///         Ok(true)
///     }
///
///     fn event_signature(&self) -> Option<B256> {
///         // Return None to process all events from the contract
///         None
///     }
/// }
/// ```
#[async_trait::async_trait]
pub trait MessageHandler: Send + Sync + Debug {
    /// Process a raw log from the blockchain.
    ///
    /// This method is called for each event that matches the filter criteria.
    /// The implementation should:
    /// 1. Decode the log data if needed
    /// 2. Perform any application-specific processing
    /// 3. Return `Ok(true)` if the event was successfully processed
    /// 4. Return `Ok(false)` if the event should not be marked as processed
    /// 5. Return `Err(_)` if processing failed and should be retried
    ///
    /// # Arguments
    ///
    /// * `log` - The raw blockchain log to process
    ///
    /// # Returns
    ///
    /// * `Ok(true)` - Event successfully processed, mark as done
    /// * `Ok(false)` - Event received but not processed, don't mark as done
    /// * `Err(_)` - Processing failed, will be retried
    async fn handle_event(&self, log: &Log) -> Result<bool>;

    /// Get the event signature this handler is interested in.
    ///
    /// If this returns `Some(signature)`, the monitor will only deliver events
    /// with this specific signature. If it returns `None`, all events from the
    /// monitored contract will be delivered.
    ///
    /// The event signature can be obtained from event definitions:
    /// ```ignore
    /// use alloy::sol;
    ///
    /// sol! {
    ///     event Deposit(address indexed from, uint256 amount);
    /// }
    ///
    /// let signature = Deposit::SIGNATURE_HASH;
    /// ```
    fn event_signature(&self) -> Option<B256>;

    /// Called when the monitor starts.
    ///
    /// Override this to perform any initialization needed before event
    /// processing begins.
    async fn on_start(&self) -> Result<()> {
        Ok(())
    }

    /// Called when the monitor stops.
    ///
    /// Override this to perform any cleanup needed when event processing ends.
    async fn on_stop(&self) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct TestHandler {
        event_count: std::sync::Arc<std::sync::atomic::AtomicU64>,
    }

    #[async_trait::async_trait]
    impl MessageHandler for TestHandler {
        async fn handle_event(&self, _log: &Log) -> Result<bool> {
            self.event_count
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(true)
        }

        fn event_signature(&self) -> Option<B256> {
            None
        }
    }

    #[tokio::test]
    async fn test_handler_trait() {
        let handler = TestHandler {
            event_count: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
        };

        // Create a dummy log
        let log = Log::default();

        // Process event
        let result = handler.handle_event(&log).await;
        assert!(result.is_ok());
        assert!(result.unwrap());

        // Verify count incremented
        assert_eq!(
            handler
                .event_count
                .load(std::sync::atomic::Ordering::SeqCst),
            1
        );
    }

    #[tokio::test]
    async fn test_handler_lifecycle() {
        let handler = TestHandler {
            event_count: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
        };

        // Test lifecycle methods
        assert!(handler.on_start().await.is_ok());
        assert!(handler.on_stop().await.is_ok());
    }
}
