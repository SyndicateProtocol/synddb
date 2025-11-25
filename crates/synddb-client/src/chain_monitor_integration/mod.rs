//! Chain monitor integration
//!
//! This module provides a unified interface for chain monitoring functionality,
//! with implementations that vary based on the "chain-monitor" feature flag.

#[cfg(feature = "chain-monitor")]
pub mod active;
#[cfg(not(feature = "chain-monitor"))]
pub mod stub;

#[cfg(feature = "chain-monitor")]
pub use active::ChainMonitorHandle;
#[cfg(not(feature = "chain-monitor"))]
pub use stub::ChainMonitorHandle;
