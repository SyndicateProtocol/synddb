//! Generic blockchain event monitoring for `SyndDB`
//!
//! This crate provides a generic chain monitoring service that can listen to
//! any blockchain contract events via WebSocket subscriptions or RPC polling.
//! It uses a trait-based architecture to allow applications to define custom
//! event processing logic.
//!
//! # Architecture
//!
//! - `ChainMonitor`: Main service that orchestrates blockchain listening
//! - `EthClient`: Robust Ethereum client wrapper with retry logic
//! - `MultiRpcProvider`: Automatic failover between multiple RPC endpoints
//! - `MessageHandler`: Trait for custom event processing logic
//! - `EventStore`: SQLite-based persistence for idempotency and crash recovery
//!
//! # Example
//!
//! ```ignore
//! // Full example will be available after Phase 3+ implementation
//! use synddb_chain_monitor::{ChainMonitor, ChainMonitorConfig, MessageHandler};
//! use alloy::rpc::types::Log;
//! use anyhow::Result;
//!
//! struct MyHandler;
//!
//! #[async_trait::async_trait]
//! impl MessageHandler for MyHandler {
//!     async fn handle_event(&self, log: &Log) -> Result<bool> {
//!         // Custom event processing logic
//!         println!("Received event: {:?}", log);
//!         Ok(true)
//!     }
//!
//!     fn event_signature(&self) -> Option<alloy::primitives::B256> {
//!         None // Process all events
//!     }
//! }
//! ```

pub mod config;
pub mod eth_client;
pub mod event_store;
pub mod events;
pub mod handler;
pub mod monitor;

// Re-export main types
pub use config::ChainMonitorConfig;
pub use eth_client::EthClient;
pub use event_store::EventStore;
pub use handler::MessageHandler;
pub use monitor::ChainMonitor;

// Re-export commonly used Alloy types for convenience
pub use alloy::{
    primitives::{Address, B256},
    rpc::types::{Filter, Log},
};

// Re-export example events for convenience
pub use events::{Deposit, OwnershipTransferred, StateSync, Withdrawal};
