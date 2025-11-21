//! Application alerting mechanisms

use super::degradation::SystemStatus;
use anyhow::Result;

#[derive(Debug)]
pub struct AlertManager {
    // TODO: Add alert state
}

impl AlertManager {
    pub const fn new() -> Self {
        Self {}
    }

    /// Send alert to application
    pub async fn send_alert(&self, status: SystemStatus, message: &str) -> Result<()> {
        // TODO: Implement alerting
        // - HTTP webhook
        // - Write to special alert table
        // - Log to monitoring system
        tracing::warn!("ALERT: {:?} - {}", status, message);
        Ok(())
    }
}

impl Default for AlertManager {
    fn default() -> Self {
        Self::new()
    }
}
