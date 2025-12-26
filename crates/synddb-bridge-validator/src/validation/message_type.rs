use crate::error::ValidationError;
use crate::types::{Message, MessageTypeConfig};

pub struct MessageTypeValidator;

impl MessageTypeValidator {
    pub fn new() -> Self {
        Self
    }

    pub fn validate(
        &self,
        message: &Message,
        config: &MessageTypeConfig,
    ) -> Result<(), ValidationError> {
        // Check if message type matches
        if config.message_type != message.message_type {
            return Err(ValidationError::MessageTypeNotRegistered(
                message.message_type.clone(),
            ));
        }

        // Check if message type is enabled
        if !config.enabled {
            return Err(ValidationError::MessageTypeInactive(
                message.message_type.clone(),
            ));
        }

        Ok(())
    }
}

impl Default for MessageTypeValidator {
    fn default() -> Self {
        Self::new()
    }
}
