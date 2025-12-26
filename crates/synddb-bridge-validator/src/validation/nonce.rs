use crate::error::ValidationError;
use crate::state::NonceStore;
use crate::types::Message;

pub struct NonceValidator<'a> {
    nonce_store: &'a NonceStore,
}

impl<'a> NonceValidator<'a> {
    pub fn new(nonce_store: &'a NonceStore) -> Self {
        Self { nonce_store }
    }

    pub fn validate(&self, message: &Message) -> Result<(), ValidationError> {
        let expected = self
            .nonce_store
            .get_expected_nonce(&message.domain)
            .map_err(|e| ValidationError::Internal(e.to_string()))?;

        if message.nonce != expected {
            return Err(ValidationError::InvalidNonce {
                domain: hex::encode(message.domain),
                expected,
                provided: message.nonce,
            });
        }

        Ok(())
    }
}
