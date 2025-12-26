//! Binary Prediction Market using Message-Passing Bridge Paradigm
//!
//! This example demonstrates the ergonomic differences between the message-passing
//! approach (this crate) and the direct SQLite approach (see `example-app` branch).
//!
//! ## Architecture
//!
//! - **Writes** go through Bridge messages to the validator
//! - **Reads** come from a local SQLite cache synced from on-chain events
//! - Demonstrates polling, retry logic, and eventual consistency challenges
//!
//! ## Key Differences from SQLite Approach
//!
//! | Aspect | SQLite | Message-Passing |
//! |--------|--------|-----------------|
//! | Write latency | Instant | Seconds (validator + chain) |
//! | Read consistency | Strong | Eventual (cache lag) |
//! | Error handling | ACID rollback | Retry + polling |
//! | Complexity | Low | High (sync, retry, timeout) |

pub mod bridge;
pub mod schema;
pub mod store;
pub mod sync;
pub mod types;

pub use types::{Account, Market, Outcome, Position, Trade, TradeSide};
