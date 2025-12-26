use crate::{error::ValidationError, state::MessageStore, types::Message};

#[derive(Debug)]
pub struct ReplayValidator<'a> {
    message_store: &'a MessageStore,
}

impl<'a> ReplayValidator<'a> {
    pub const fn new(message_store: &'a MessageStore) -> Self {
        Self { message_store }
    }

    pub fn validate(&self, message: &Message) -> Result<(), ValidationError> {
        let is_processed = self
            .message_store
            .is_processed(&message.id)
            .map_err(|e| ValidationError::Internal(e.to_string()))?;

        if is_processed {
            return Err(ValidationError::ReplayDetected(hex::encode(message.id)));
        }

        Ok(())
    }
}
