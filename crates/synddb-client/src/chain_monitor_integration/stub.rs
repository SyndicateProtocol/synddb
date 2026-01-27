//! Stub chain monitor implementation (when "chain-monitor" feature is disabled)

use anyhow::Result;
use rusqlite::Connection;
use synddb_chain_monitor::config::ChainMonitorConfig;

/// No-op chain monitor handle
///
/// This is a stub implementation that does nothing. It's used when the
/// "chain-monitor" feature is not enabled, allowing the code to compile
/// without feature flags scattered throughout.
#[derive(Debug)]
pub struct ChainMonitorHandle;

impl ChainMonitorHandle {
    /// Create a stub chain monitor (no-op)
    ///
    /// This always returns an error indicating the feature is not enabled.
    pub fn new(_config: ChainMonitorConfig, _conn: &'static Connection) -> Result<Self> {
        Err(anyhow::anyhow!(
            "Chain monitor feature not enabled. Rebuild with --features chain-monitor"
        ))
    }

    /// Process deposits (no-op)
    ///
    /// Always returns 0 since no monitoring is active.
    #[allow(clippy::missing_const_for_fn)] // Result type is not const-compatible
    pub fn process_deposits(&self) -> Result<usize> {
        Ok(0)
    }
}
