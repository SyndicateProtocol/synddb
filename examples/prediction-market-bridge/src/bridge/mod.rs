//! Bridge integration module.
//!
//! Provides the client for submitting messages to the Bridge validator
//! and tracking their status.

pub mod client;
pub mod encoding;
pub mod types;

pub use client::BridgeClient;
pub use types::{MessageStatus, PushResult};
