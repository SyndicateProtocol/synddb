use alloy::primitives::Address;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplicationConfig {
    pub domain: [u8; 32],
    pub primary_validator: Address,
    pub expiration_seconds: u64,
    pub require_witness_signatures: bool,
    pub active: bool,
}

impl ApplicationConfig {
    pub fn is_valid(&self) -> bool {
        self.active && self.primary_validator != Address::ZERO
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageTypeConfig {
    pub message_type: String,
    pub target: Address,
    pub schema_hash: [u8; 32],
    pub schema_uri: String,
    pub active: bool,
}

impl MessageTypeConfig {
    pub fn is_valid(&self) -> bool {
        self.active && self.target != Address::ZERO
    }

    pub fn has_schema(&self) -> bool {
        self.schema_hash != [0u8; 32] || !self.schema_uri.is_empty()
    }
}
