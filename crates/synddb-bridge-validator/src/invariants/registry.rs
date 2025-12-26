use alloy::providers::ProviderBuilder;
use async_trait::async_trait;

use crate::error::ValidationError;
use crate::types::Message;

pub struct InvariantContext {
    rpc_url: Option<String>,
}

impl InvariantContext {
    pub fn new() -> Self {
        Self { rpc_url: None }
    }

    pub fn with_rpc_url(rpc_url: String) -> Self {
        Self {
            rpc_url: Some(rpc_url),
        }
    }

    pub fn rpc_url(&self) -> Option<&str> {
        self.rpc_url.as_deref()
    }

    pub fn create_provider(&self) -> Option<impl alloy::providers::Provider + Clone> {
        let url = self.rpc_url.as_ref()?;
        let parsed_url: reqwest::Url = url.parse().ok()?;
        Some(ProviderBuilder::new().connect_http(parsed_url))
    }
}

impl Default for InvariantContext {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
pub trait Invariant: Send + Sync {
    fn name(&self) -> &str;

    async fn check(
        &self,
        message: &Message,
        ctx: &InvariantContext,
    ) -> Result<(), ValidationError>;
}

pub struct InvariantRegistry {
    invariants: Vec<Box<dyn Invariant>>,
}

impl InvariantRegistry {
    pub fn new() -> Self {
        Self {
            invariants: Vec::new(),
        }
    }

    pub fn register(&mut self, invariant: Box<dyn Invariant>) {
        self.invariants.push(invariant);
    }

    pub async fn check_all(
        &self,
        message: &Message,
        ctx: &InvariantContext,
    ) -> Result<(), ValidationError> {
        for invariant in &self.invariants {
            invariant.check(message, ctx).await?;
        }
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.invariants.len()
    }

    pub fn is_empty(&self) -> bool {
        self.invariants.is_empty()
    }
}

impl Default for InvariantRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct AlwaysPassInvariant;

    #[async_trait]
    impl Invariant for AlwaysPassInvariant {
        fn name(&self) -> &str {
            "always_pass"
        }

        async fn check(
            &self,
            _message: &Message,
            _ctx: &InvariantContext,
        ) -> Result<(), ValidationError> {
            Ok(())
        }
    }

    struct AlwaysFailInvariant;

    #[async_trait]
    impl Invariant for AlwaysFailInvariant {
        fn name(&self) -> &str {
            "always_fail"
        }

        async fn check(
            &self,
            _message: &Message,
            _ctx: &InvariantContext,
        ) -> Result<(), ValidationError> {
            Err(ValidationError::InvariantViolated {
                invariant: "always_fail".to_string(),
                message: "This invariant always fails".to_string(),
            })
        }
    }

    #[tokio::test]
    async fn test_invariant_registry_pass() {
        let mut registry = InvariantRegistry::new();
        registry.register(Box::new(AlwaysPassInvariant));

        let message = Message {
            id: [0u8; 32],
            message_type: "test()".to_string(),
            calldata: vec![],
            metadata: serde_json::Value::Null,
            metadata_hash: [0u8; 32],
            nonce: 1,
            timestamp: 1234567890,
            domain: [0u8; 32],
            value: None,
        };

        let ctx = InvariantContext::new();
        assert!(registry.check_all(&message, &ctx).await.is_ok());
    }

    #[tokio::test]
    async fn test_invariant_registry_fail() {
        let mut registry = InvariantRegistry::new();
        registry.register(Box::new(AlwaysPassInvariant));
        registry.register(Box::new(AlwaysFailInvariant));

        let message = Message {
            id: [0u8; 32],
            message_type: "test()".to_string(),
            calldata: vec![],
            metadata: serde_json::Value::Null,
            metadata_hash: [0u8; 32],
            nonce: 1,
            timestamp: 1234567890,
            domain: [0u8; 32],
            value: None,
        };

        let ctx = InvariantContext::new();
        let result = registry.check_all(&message, &ctx).await;
        assert!(result.is_err());
    }
}
