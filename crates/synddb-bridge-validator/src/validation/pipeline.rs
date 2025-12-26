use std::sync::Arc;
use std::time::Duration;

use crate::bridge::BridgeClient;
use crate::error::ValidationError;
use crate::invariants::{InvariantContext, InvariantRegistry};
use crate::state::{MessageStore, NonceStore};
use crate::types::{ApplicationConfig, Message, MessageTypeConfig};

use super::{
    AppAuthValidator, CalldataValidator, MessageTypeValidator, NonceValidator, ReplayValidator,
    SchemaValidator, TimestampValidator,
};

pub struct ValidationContext {
    pub app_config: ApplicationConfig,
    pub message_type_config: MessageTypeConfig,
    pub schema: Option<serde_json::Value>,
}

pub struct ValidationPipeline {
    message_store: Arc<MessageStore>,
    nonce_store: Arc<NonceStore>,
    timestamp_validator: TimestampValidator,
    schema_validator: SchemaValidator,
    invariant_registry: InvariantRegistry,
}

impl ValidationPipeline {
    pub fn new(
        message_store: Arc<MessageStore>,
        nonce_store: Arc<NonceStore>,
        max_clock_drift: Duration,
        schema_cache_ttl: Duration,
    ) -> Self {
        Self {
            message_store,
            nonce_store,
            timestamp_validator: TimestampValidator::new(max_clock_drift),
            schema_validator: SchemaValidator::new(schema_cache_ttl),
            invariant_registry: InvariantRegistry::new(),
        }
    }

    pub fn register_invariant(&mut self, invariant: Box<dyn crate::invariants::Invariant>) {
        self.invariant_registry.register(invariant);
    }

    pub async fn validate(
        &self,
        message: &Message,
        ctx: &ValidationContext,
    ) -> Result<(), ValidationError> {
        // Stage 1: Replay protection
        tracing::debug!(message_id = %hex::encode(message.id), "Stage 1: Replay check");
        ReplayValidator::new(&self.message_store).validate(message)?;

        // Stage 2: Nonce validation
        tracing::debug!(message_id = %hex::encode(message.id), nonce = message.nonce, "Stage 2: Nonce check");
        NonceValidator::new(&self.nonce_store).validate(message)?;

        // Stage 3: Timestamp freshness
        tracing::debug!(message_id = %hex::encode(message.id), timestamp = message.timestamp, "Stage 3: Timestamp check");
        self.timestamp_validator.validate(message)?;

        // Stage 4: App authorization
        tracing::debug!(message_id = %hex::encode(message.id), domain = %hex::encode(message.domain), "Stage 4: App auth check");
        AppAuthValidator::new().validate(message, &ctx.app_config)?;

        // Stage 5: Message type validation
        tracing::debug!(message_id = %hex::encode(message.id), message_type = %message.message_type, "Stage 5: Message type check");
        MessageTypeValidator::new().validate(message, &ctx.message_type_config)?;

        // Stage 6: Calldata validation (ABI decode)
        tracing::debug!(message_id = %hex::encode(message.id), "Stage 6: Calldata check");
        CalldataValidator::new().validate(message)?;

        // Stage 7: Schema validation
        tracing::debug!(message_id = %hex::encode(message.id), "Stage 7: Schema check");
        self.schema_validator
            .validate(message, ctx.schema.as_ref())?;

        // Stage 8: Invariant checking
        tracing::debug!(message_id = %hex::encode(message.id), "Stage 8: Invariant check");
        let invariant_ctx = InvariantContext::new();
        self.invariant_registry
            .check_all(message, &invariant_ctx)
            .await?;

        // Stage 9: Custom rules (rate limits, thresholds) - placeholder
        tracing::debug!(message_id = %hex::encode(message.id), "Stage 9: Custom rules check");
        // No custom rules implemented yet

        tracing::info!(message_id = %hex::encode(message.id), "All validation stages passed");
        Ok(())
    }

    pub async fn fetch_context(
        &self,
        message: &Message,
        bridge_client: &BridgeClient,
    ) -> Result<ValidationContext, ValidationError> {
        // Fetch application config
        let app_config = bridge_client
            .get_application_config(message.domain)
            .await
            .map_err(|e| ValidationError::BridgeConnectionFailed(e.to_string()))?;

        // Fetch message type config
        let message_type_config = bridge_client
            .get_message_type_config(&message.message_type)
            .await
            .map_err(|e| ValidationError::BridgeConnectionFailed(e.to_string()))?;

        // TODO: Fetch schema from schema_uri if present
        let schema = None;

        Ok(ValidationContext {
            app_config,
            message_type_config,
            schema,
        })
    }

    pub fn consume_nonce(&self, domain: &[u8; 32], nonce: u64) -> Result<(), ValidationError> {
        self.nonce_store
            .consume_nonce(domain, nonce)
            .map_err(|e| ValidationError::Internal(e.to_string()))
    }

    pub fn mark_message_processed(&self, message_id: &[u8; 32]) -> Result<(), ValidationError> {
        self.message_store
            .mark_processed(message_id)
            .map_err(|e| ValidationError::Internal(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::Address;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn make_test_message() -> Message {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        Message {
            id: [1u8; 32],
            message_type: "setValue(uint256)".to_string(),
            calldata: hex::decode(
                "55241077000000000000000000000000000000000000000000000000000000000000002a",
            )
            .unwrap(),
            metadata: serde_json::json!({}),
            metadata_hash: [0u8; 32],
            nonce: 1,
            timestamp: now,
            domain: [0u8; 32],
            value: None,
        }
    }

    fn make_test_context() -> ValidationContext {
        ValidationContext {
            app_config: ApplicationConfig {
                domain: [0u8; 32],
                primary_validator: Address::ZERO,
                expiration_seconds: 86400,
                require_witness_signatures: true,
                active: true,
            },
            message_type_config: MessageTypeConfig {
                message_type: "setValue(uint256)".to_string(),
                target: Address::ZERO,
                schema_hash: [0u8; 32],
                schema_uri: String::new(),
                active: true,
            },
            schema: None,
        }
    }

    #[tokio::test]
    async fn test_pipeline_all_stages_pass() {
        let message_store = Arc::new(MessageStore::new(":memory:").unwrap());
        let nonce_store = Arc::new(NonceStore::new(":memory:").unwrap());

        let pipeline = ValidationPipeline::new(
            message_store,
            nonce_store,
            Duration::from_secs(60),
            Duration::from_secs(3600),
        );

        let message = make_test_message();
        let ctx = make_test_context();

        let result = pipeline.validate(&message, &ctx).await;
        assert!(result.is_ok(), "Expected Ok, got {:?}", result);
    }

    #[tokio::test]
    async fn test_pipeline_replay_detected() {
        let message_store = Arc::new(MessageStore::new(":memory:").unwrap());
        let nonce_store = Arc::new(NonceStore::new(":memory:").unwrap());

        // Mark message as already processed
        let message = make_test_message();
        message_store.mark_processed(&message.id).unwrap();

        let pipeline = ValidationPipeline::new(
            message_store,
            nonce_store,
            Duration::from_secs(60),
            Duration::from_secs(3600),
        );

        let ctx = make_test_context();
        let result = pipeline.validate(&message, &ctx).await;

        assert!(matches!(result, Err(ValidationError::ReplayDetected(_))));
    }

    #[tokio::test]
    async fn test_pipeline_invalid_nonce() {
        let message_store = Arc::new(MessageStore::new(":memory:").unwrap());
        let nonce_store = Arc::new(NonceStore::new(":memory:").unwrap());

        // Consume nonce 1, so expected is now 2
        nonce_store.consume_nonce(&[0u8; 32], 1).unwrap();

        let pipeline = ValidationPipeline::new(
            message_store,
            nonce_store,
            Duration::from_secs(60),
            Duration::from_secs(3600),
        );

        let message = make_test_message(); // nonce = 1, but expected = 2
        let ctx = make_test_context();

        let result = pipeline.validate(&message, &ctx).await;
        assert!(matches!(result, Err(ValidationError::InvalidNonce { .. })));
    }

    #[tokio::test]
    async fn test_pipeline_inactive_app() {
        let message_store = Arc::new(MessageStore::new(":memory:").unwrap());
        let nonce_store = Arc::new(NonceStore::new(":memory:").unwrap());

        let pipeline = ValidationPipeline::new(
            message_store,
            nonce_store,
            Duration::from_secs(60),
            Duration::from_secs(3600),
        );

        let message = make_test_message();
        let mut ctx = make_test_context();
        ctx.app_config.active = false;

        let result = pipeline.validate(&message, &ctx).await;
        assert!(matches!(result, Err(ValidationError::AppInactive(_))));
    }
}
