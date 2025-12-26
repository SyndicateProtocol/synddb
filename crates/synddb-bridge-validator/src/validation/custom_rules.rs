use std::{
    collections::HashMap,
    sync::RwLock,
    time::{Duration, Instant},
};

use crate::{error::ValidationError, types::Message};

#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    pub max_per_second: Option<u32>,
    pub max_per_minute: Option<u32>,
    pub max_per_hour: Option<u32>,
    pub max_value_per_hour: Option<u128>,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            max_per_second: Some(10),
            max_per_minute: Some(100),
            max_per_hour: Some(1000),
            max_value_per_hour: None,
        }
    }
}

pub struct CustomRulesValidator {
    rate_limits: RwLock<HashMap<[u8; 32], DomainRateLimiter>>,
    default_config: RateLimitConfig,
    domain_configs: HashMap<[u8; 32], RateLimitConfig>,
}

struct DomainRateLimiter {
    second_window: SlidingWindow,
    minute_window: SlidingWindow,
    hour_window: SlidingWindow,
    hour_value: ValueWindow,
}

struct SlidingWindow {
    timestamps: Vec<Instant>,
    duration: Duration,
}

impl SlidingWindow {
    fn new(duration: Duration) -> Self {
        Self {
            timestamps: Vec::new(),
            duration,
        }
    }

    fn count(&mut self) -> usize {
        let now = Instant::now();
        self.timestamps
            .retain(|t| now.duration_since(*t) < self.duration);
        self.timestamps.len()
    }

    fn record(&mut self) {
        self.timestamps.push(Instant::now());
    }
}

struct ValueWindow {
    entries: Vec<(Instant, u128)>,
    duration: Duration,
}

impl ValueWindow {
    fn new(duration: Duration) -> Self {
        Self {
            entries: Vec::new(),
            duration,
        }
    }

    fn total(&mut self) -> u128 {
        let now = Instant::now();
        self.entries
            .retain(|(t, _)| now.duration_since(*t) < self.duration);
        self.entries.iter().map(|(_, v)| v).sum()
    }

    fn record(&mut self, value: u128) {
        self.entries.push((Instant::now(), value));
    }
}

impl DomainRateLimiter {
    fn new() -> Self {
        Self {
            second_window: SlidingWindow::new(Duration::from_secs(1)),
            minute_window: SlidingWindow::new(Duration::from_secs(60)),
            hour_window: SlidingWindow::new(Duration::from_secs(3600)),
            hour_value: ValueWindow::new(Duration::from_secs(3600)),
        }
    }
}

impl CustomRulesValidator {
    pub fn new(default_config: RateLimitConfig) -> Self {
        Self {
            rate_limits: RwLock::new(HashMap::new()),
            default_config,
            domain_configs: HashMap::new(),
        }
    }

    pub fn with_domain_config(mut self, domain: [u8; 32], config: RateLimitConfig) -> Self {
        self.domain_configs.insert(domain, config);
        self
    }

    pub fn validate(&self, message: &Message) -> Result<(), ValidationError> {
        let config = self
            .domain_configs
            .get(&message.domain)
            .unwrap_or(&self.default_config);

        let mut limits = self.rate_limits.write().unwrap();
        let limiter = limits
            .entry(message.domain)
            .or_insert_with(DomainRateLimiter::new);

        // Check per-second limit
        if let Some(max) = config.max_per_second {
            let count = limiter.second_window.count();
            if count >= max as usize {
                return Err(ValidationError::RateLimitExceeded(format!(
                    "Exceeded {} messages per second (current: {})",
                    max, count
                )));
            }
        }

        // Check per-minute limit
        if let Some(max) = config.max_per_minute {
            let count = limiter.minute_window.count();
            if count >= max as usize {
                return Err(ValidationError::RateLimitExceeded(format!(
                    "Exceeded {} messages per minute (current: {})",
                    max, count
                )));
            }
        }

        // Check per-hour limit
        if let Some(max) = config.max_per_hour {
            let count = limiter.hour_window.count();
            if count >= max as usize {
                return Err(ValidationError::RateLimitExceeded(format!(
                    "Exceeded {} messages per hour (current: {})",
                    max, count
                )));
            }
        }

        // Check value limit per hour
        if let Some(max_value) = config.max_value_per_hour {
            if let Some(msg_value) = message.value {
                let current_total = limiter.hour_value.total();
                if current_total + msg_value > max_value {
                    return Err(ValidationError::RateLimitExceeded(format!(
                        "Exceeded {} wei value per hour (current: {}, requested: {})",
                        max_value, current_total, msg_value
                    )));
                }
            }
        }

        Ok(())
    }

    pub fn record(&self, message: &Message) {
        let mut limits = self.rate_limits.write().unwrap();
        let limiter = limits
            .entry(message.domain)
            .or_insert_with(DomainRateLimiter::new);

        limiter.second_window.record();
        limiter.minute_window.record();
        limiter.hour_window.record();

        if let Some(value) = message.value {
            limiter.hour_value.record(value);
        }
    }

    pub fn clear_domain(&self, domain: &[u8; 32]) {
        let mut limits = self.rate_limits.write().unwrap();
        limits.remove(domain);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_message(domain: [u8; 32], value: Option<u128>) -> Message {
        Message {
            id: [0u8; 32],
            message_type: "test()".to_string(),
            calldata: vec![],
            metadata: json!({}),
            metadata_hash: [0u8; 32],
            nonce: 1,
            timestamp: 1234567890,
            domain,
            value,
        }
    }

    #[test]
    fn test_rate_limit_pass() {
        let config = RateLimitConfig {
            max_per_second: Some(10),
            max_per_minute: Some(100),
            max_per_hour: Some(1000),
            max_value_per_hour: None,
        };

        let validator = CustomRulesValidator::new(config);
        let message = make_message([0u8; 32], None);

        assert!(validator.validate(&message).is_ok());
    }

    #[test]
    fn test_rate_limit_exceeded_per_second() {
        let config = RateLimitConfig {
            max_per_second: Some(2),
            max_per_minute: None,
            max_per_hour: None,
            max_value_per_hour: None,
        };

        let validator = CustomRulesValidator::new(config);
        let message = make_message([0u8; 32], None);

        // Record 2 messages
        validator.record(&message);
        validator.record(&message);

        // Third should fail
        let result = validator.validate(&message);
        assert!(matches!(result, Err(ValidationError::RateLimitExceeded(_))));
    }

    #[test]
    fn test_value_limit_exceeded() {
        let config = RateLimitConfig {
            max_per_second: None,
            max_per_minute: None,
            max_per_hour: None,
            max_value_per_hour: Some(1000),
        };

        let validator = CustomRulesValidator::new(config);
        let message = make_message([0u8; 32], Some(600));

        // Record first 600 wei
        validator.record(&message);

        // Second 600 wei should fail (total would be 1200 > 1000)
        let result = validator.validate(&message);
        assert!(matches!(result, Err(ValidationError::RateLimitExceeded(_))));
    }

    #[test]
    fn test_per_domain_config() {
        let default_config = RateLimitConfig {
            max_per_second: Some(1),
            max_per_minute: None,
            max_per_hour: None,
            max_value_per_hour: None,
        };

        let custom_config = RateLimitConfig {
            max_per_second: Some(100),
            max_per_minute: None,
            max_per_hour: None,
            max_value_per_hour: None,
        };

        let custom_domain = [1u8; 32];
        let validator = CustomRulesValidator::new(default_config)
            .with_domain_config(custom_domain, custom_config);

        let default_message = make_message([0u8; 32], None);
        let custom_message = make_message(custom_domain, None);

        // Default domain should hit limit after 1
        validator.record(&default_message);
        assert!(validator.validate(&default_message).is_err());

        // Custom domain should allow many more
        for _ in 0..50 {
            validator.record(&custom_message);
        }
        assert!(validator.validate(&custom_message).is_ok());
    }
}
