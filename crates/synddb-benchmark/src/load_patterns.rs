use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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
    pub batch_size: usize,
    pub simple_mode: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_pattern_continuous() {
        let pattern = LoadPattern::Continuous {
            ops_per_second: 100,
        };

        match pattern {
            LoadPattern::Continuous { ops_per_second } => {
                assert_eq!(ops_per_second, 100);
            }
            _ => panic!("Expected Continuous pattern"),
        }
    }

    #[test]
    fn test_load_pattern_burst() {
        let pattern = LoadPattern::Burst {
            burst_size: 1000,
            pause_seconds: 5,
        };

        match pattern {
            LoadPattern::Burst {
                burst_size,
                pause_seconds,
            } => {
                assert_eq!(burst_size, 1000);
                assert_eq!(pause_seconds, 5);
            }
            _ => panic!("Expected Burst pattern"),
        }
    }

    #[test]
    fn test_load_config_with_duration() {
        let config = LoadConfig {
            pattern: LoadPattern::Continuous { ops_per_second: 50 },
            duration_seconds: Some(60),
            batch_size: 100,
            simple_mode: false,
        };

        assert_eq!(config.duration_seconds, Some(60));
        assert_eq!(config.batch_size, 100);
        assert_eq!(config.simple_mode, false);
    }

    #[test]
    fn test_load_config_without_duration() {
        let config = LoadConfig {
            pattern: LoadPattern::Continuous { ops_per_second: 50 },
            duration_seconds: None,
            batch_size: 50,
            simple_mode: false,
        };

        assert_eq!(config.duration_seconds, None);
        assert_eq!(config.batch_size, 50);
        assert_eq!(config.simple_mode, false);
    }

    #[test]
    fn test_load_pattern_serialization() {
        let pattern = LoadPattern::Continuous {
            ops_per_second: 200,
        };
        let json = serde_json::to_string(&pattern).unwrap();
        let deserialized: LoadPattern = serde_json::from_str(&json).unwrap();

        assert_eq!(pattern, deserialized);
    }

    #[test]
    fn test_burst_pattern_serialization() {
        let pattern = LoadPattern::Burst {
            burst_size: 500,
            pause_seconds: 10,
        };
        let json = serde_json::to_string(&pattern).unwrap();
        let deserialized: LoadPattern = serde_json::from_str(&json).unwrap();

        assert_eq!(pattern, deserialized);
    }
}
