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
//! - `MessageHandler`: Trait for custom event processing logic
//! - `EventStore`: `SQLite`-based persistence for idempotency and crash recovery
//!
//! See the [examples directory](https://github.com/SyndicateProtocol/SyndDB/tree/main/crates/synddb-chain-monitor/examples)
//! for detailed usage patterns including deposit handling, multi-event routing, and production configurations.

pub mod config;
pub mod eth_client;
pub mod event_store;
pub mod events;
pub mod handler;
pub mod monitor;
