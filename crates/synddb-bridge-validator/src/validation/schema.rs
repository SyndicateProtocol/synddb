use std::{
    collections::HashMap,
    sync::RwLock,
    time::{Duration, Instant},
};

use anyhow::Result;
use jsonschema::Validator;

use crate::{error::ValidationError, types::Message};

pub struct SchemaValidator {
    cache: RwLock<HashMap<String, CachedSchema>>,
    ttl: Duration,
}

struct CachedSchema {
    validator: Validator,
    cached_at: Instant,
}

impl SchemaValidator {
    pub fn new(ttl: Duration) -> Self {
        Self {
            cache: RwLock::new(HashMap::new()),
            ttl,
        }
    }

    pub fn validate(
        &self,
        message: &Message,
        schema: Option<&serde_json::Value>,
    ) -> Result<(), ValidationError> {
        let Some(schema) = schema else {
            // No schema defined, skip validation
            return Ok(());
        };

        // Check cache first
        {
            let cache = self.cache.read().unwrap();
            if let Some(cached) = cache.get(&message.message_type) {
                if cached.cached_at.elapsed() < self.ttl {
                    return self.validate_with_validator(&cached.validator, message);
                }
            }
        }

        // Cache miss or expired, compile schema
        let validator = Validator::new(schema).map_err(|e| {
            ValidationError::SchemaValidationFailed(format!("Invalid schema: {}", e))
        })?;

        let result = self.validate_with_validator(&validator, message);

        // Update cache
        {
            let mut cache = self.cache.write().unwrap();
            cache.insert(
                message.message_type.clone(),
                CachedSchema {
                    validator,
                    cached_at: Instant::now(),
                },
            );
        }

        result
    }

    fn validate_with_validator(
        &self,
        validator: &Validator,
        message: &Message,
    ) -> Result<(), ValidationError> {
        if !validator.is_valid(&message.metadata) {
            let errors: Vec<String> = validator
                .iter_errors(&message.metadata)
                .map(|e| e.to_string())
                .collect();

            return Err(ValidationError::SchemaValidationFailed(errors.join("; ")));
        }

        Ok(())
    }

    pub fn invalidate(&self, message_type: &str) {
        let mut cache = self.cache.write().unwrap();
        cache.remove(message_type);
    }

    pub fn clear_cache(&self) {
        let mut cache = self.cache.write().unwrap();
        cache.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_message(metadata: serde_json::Value) -> Message {
        Message {
            id: [0u8; 32],
            message_type: "test()".to_string(),
            calldata: vec![],
            metadata,
            metadata_hash: [0u8; 32],
            nonce: 1,
            timestamp: 1234567890,
            domain: [0u8; 32],
            value: None,
        }
    }

    #[test]
    fn test_no_schema() {
        let validator = SchemaValidator::new(Duration::from_secs(3600));
        let message = make_message(json!({"key": "value"}));

        assert!(validator.validate(&message, None).is_ok());
    }

    #[test]
    fn test_valid_schema() {
        let validator = SchemaValidator::new(Duration::from_secs(3600));
        let schema = json!({
            "type": "object",
            "properties": {
                "amount": {"type": "integer"},
                "recipient": {"type": "string"}
            },
            "required": ["amount", "recipient"]
        });

        let message = make_message(json!({
            "amount": 100,
            "recipient": "0x1234"
        }));

        assert!(validator.validate(&message, Some(&schema)).is_ok());
    }

    #[test]
    fn test_invalid_schema() {
        let validator = SchemaValidator::new(Duration::from_secs(3600));
        let schema = json!({
            "type": "object",
            "properties": {
                "amount": {"type": "integer"}
            },
            "required": ["amount"]
        });

        let message = make_message(json!({
            "amount": "not a number"
        }));

        assert!(matches!(
            validator.validate(&message, Some(&schema)),
            Err(ValidationError::SchemaValidationFailed(_))
        ));
    }

    #[test]
    fn test_missing_required_field() {
        let validator = SchemaValidator::new(Duration::from_secs(3600));
        let schema = json!({
            "type": "object",
            "required": ["amount"]
        });

        let message = make_message(json!({}));

        assert!(matches!(
            validator.validate(&message, Some(&schema)),
            Err(ValidationError::SchemaValidationFailed(_))
        ));
    }
}
