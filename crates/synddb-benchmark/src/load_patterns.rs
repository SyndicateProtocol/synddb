use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LoadPattern {
    /// Run continuously at a specified rate
    Continuous { ops_per_second: u64 },
    /// Generate bursts of activity with pauses between
    Burst {
        burst_size: usize,
        pause_seconds: u64,
    },
}

#[derive(Debug, Clone)]
pub struct LoadConfig {
    pub pattern: LoadPattern,
    pub duration_seconds: Option<u64>,
}
