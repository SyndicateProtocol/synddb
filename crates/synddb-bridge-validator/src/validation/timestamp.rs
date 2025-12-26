use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::{error::ValidationError, types::Message};

#[derive(Debug)]
pub struct TimestampValidator {
    max_clock_drift: Duration,
}

impl TimestampValidator {
    pub const fn new(max_clock_drift: Duration) -> Self {
        Self { max_clock_drift }
    }

    pub fn validate(&self, message: &Message) -> Result<(), ValidationError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let max_drift_secs = self.max_clock_drift.as_secs();

        // Check if timestamp is too far in the past
        if message.timestamp + max_drift_secs < now {
            return Err(ValidationError::TimestampExpired {
                timestamp: message.timestamp,
            });
        }

        // Check if timestamp is too far in the future
        if message.timestamp > now + max_drift_secs {
            return Err(ValidationError::TimestampFuture {
                timestamp: message.timestamp,
            });
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_message(timestamp: u64) -> Message {
        Message {
            id: [0u8; 32],
            message_type: "test()".to_string(),
            calldata: vec![],
            metadata: serde_json::Value::Null,
            metadata_hash: [0u8; 32],
            nonce: 1,
            timestamp,
            domain: [0u8; 32],
            value: None,
        }
    }

    #[test]
    fn test_valid_timestamp() {
        let validator = TimestampValidator::new(Duration::from_secs(60));
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let message = make_message(now);
        assert!(validator.validate(&message).is_ok());
    }

    #[test]
    fn test_expired_timestamp() {
        let validator = TimestampValidator::new(Duration::from_secs(60));
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let message = make_message(now - 120); // 2 minutes ago
        assert!(matches!(
            validator.validate(&message),
            Err(ValidationError::TimestampExpired { .. })
        ));
    }

    #[test]
    fn test_future_timestamp() {
        let validator = TimestampValidator::new(Duration::from_secs(60));
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let message = make_message(now + 120); // 2 minutes in future
        assert!(matches!(
            validator.validate(&message),
            Err(ValidationError::TimestampFuture { .. })
        ));
    }
}
