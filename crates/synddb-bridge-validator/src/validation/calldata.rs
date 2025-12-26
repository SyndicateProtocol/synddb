use crate::{error::ValidationError, types::Message};
use sha3::{Digest, Keccak256};

pub struct CalldataValidator;

impl CalldataValidator {
    pub fn new() -> Self {
        Self
    }

    pub fn validate(&self, message: &Message) -> Result<(), ValidationError> {
        // Validate that calldata starts with the correct function selector
        let selector = compute_selector(&message.message_type);

        if message.calldata.len() < 4 {
            return Err(ValidationError::CalldataInvalid(
                "calldata too short, must be at least 4 bytes".to_string(),
            ));
        }

        if message.calldata[..4] != selector {
            return Err(ValidationError::CalldataInvalid(format!(
                "selector mismatch: expected 0x{}, got 0x{}",
                hex::encode(selector),
                hex::encode(&message.calldata[..4])
            )));
        }

        Ok(())
    }
}

impl Default for CalldataValidator {
    fn default() -> Self {
        Self::new()
    }
}

fn compute_selector(signature: &str) -> [u8; 4] {
    let hash = Keccak256::digest(signature.as_bytes());
    let mut selector = [0u8; 4];
    selector.copy_from_slice(&hash[..4]);
    selector
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_selector() {
        // setValue(uint256) = 0x55241077
        let selector = compute_selector("setValue(uint256)");
        assert_eq!(hex::encode(selector), "55241077");
    }

    #[test]
    fn test_valid_calldata() {
        let validator = CalldataValidator::new();
        let message = Message {
            id: [0u8; 32],
            message_type: "setValue(uint256)".to_string(),
            calldata: hex::decode(
                "55241077000000000000000000000000000000000000000000000000000000000000002a",
            )
            .unwrap(),
            metadata: serde_json::Value::Null,
            metadata_hash: [0u8; 32],
            nonce: 1,
            timestamp: 1234567890,
            domain: [0u8; 32],
            value: None,
        };

        assert!(validator.validate(&message).is_ok());
    }

    #[test]
    fn test_invalid_selector() {
        let validator = CalldataValidator::new();
        let message = Message {
            id: [0u8; 32],
            message_type: "setValue(uint256)".to_string(),
            calldata: hex::decode(
                "ffffffff000000000000000000000000000000000000000000000000000000000000002a",
            )
            .unwrap(),
            metadata: serde_json::Value::Null,
            metadata_hash: [0u8; 32],
            nonce: 1,
            timestamp: 1234567890,
            domain: [0u8; 32],
            value: None,
        };

        assert!(matches!(
            validator.validate(&message),
            Err(ValidationError::CalldataInvalid(_))
        ));
    }

    #[test]
    fn test_calldata_too_short() {
        let validator = CalldataValidator::new();
        let message = Message {
            id: [0u8; 32],
            message_type: "setValue(uint256)".to_string(),
            calldata: vec![0x60, 0xfe, 0x47], // Only 3 bytes
            metadata: serde_json::Value::Null,
            metadata_hash: [0u8; 32],
            nonce: 1,
            timestamp: 1234567890,
            domain: [0u8; 32],
            value: None,
        };

        assert!(matches!(
            validator.validate(&message),
            Err(ValidationError::CalldataInvalid(_))
        ));
    }
}
