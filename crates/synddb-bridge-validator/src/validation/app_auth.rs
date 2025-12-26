use crate::{
    error::ValidationError,
    types::{ApplicationConfig, Message},
};

#[derive(Debug)]
pub struct AppAuthValidator;

impl AppAuthValidator {
    pub const fn new() -> Self {
        Self
    }

    pub fn validate(
        &self,
        message: &Message,
        config: &ApplicationConfig,
    ) -> Result<(), ValidationError> {
        // Check if domain matches
        if config.domain != message.domain {
            return Err(ValidationError::AppNotAuthorized(hex::encode(
                message.domain,
            )));
        }

        // Check if application is active
        if !config.active {
            return Err(ValidationError::AppInactive(hex::encode(message.domain)));
        }

        Ok(())
    }
}

impl Default for AppAuthValidator {
    fn default() -> Self {
        Self::new()
    }
}
