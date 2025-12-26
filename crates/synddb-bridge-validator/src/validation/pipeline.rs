use std::{sync::Arc, time::Duration};

use crate::{
    bridge::BridgeClient,
    error::ValidationError,
    invariants::{InvariantContext, InvariantRegistry},
    state::{MessageStore, NonceStore},
    types::{ApplicationConfig, Message, MessageTypeConfig},
};

use super::{
    AppAuthValidator, CalldataValidator, CustomRulesValidator, MessageTypeValidator,
    NonceValidator, RateLimitConfig, ReplayValidator, SchemaFetcher, SchemaValidator,
    TimestampValidator,
};

#[derive(Debug)]
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
    schema_fetcher: SchemaFetcher,
    invariant_registry: InvariantRegistry,
    custom_rules: CustomRulesValidator,
}

impl std::fmt::Debug for ValidationPipeline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ValidationPipeline")
            .field("timestamp_validator", &self.timestamp_validator)
            .field("invariant_registry", &self.invariant_registry)
            .field("custom_rules", &self.custom_rules)
            .finish_non_exhaustive()
    }
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
            schema_fetcher: SchemaFetcher::new(schema_cache_ttl),
            invariant_registry: InvariantRegistry::new(),
            custom_rules: CustomRulesValidator::new(RateLimitConfig::default()),
        }
    }

    pub fn with_rate_limit_config(mut self, config: RateLimitConfig) -> Self {
        self.custom_rules = CustomRulesValidator::new(config);
        self
    }

    pub fn with_domain_rate_limit(mut self, domain: [u8; 32], config: RateLimitConfig) -> Self {
        self.custom_rules = self.custom_rules.with_domain_config(domain, config);
        self
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

        // Stage 9: Custom rules (rate limits, thresholds)
        tracing::debug!(message_id = %hex::encode(message.id), "Stage 9: Custom rules check");
        self.custom_rules.validate(message)?;

        tracing::info!(message_id = %hex::encode(message.id), "All validation stages passed");

        // Record the message for rate limiting after successful validation
        self.custom_rules.record(message);

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

        // Fetch schema from schema_uri if present
        let schema = if message_type_config.schema_uri.is_empty() {
            None
        } else {
            // Convert schema_hash to Option for verification
            let expected_hash =
                (message_type_config.schema_hash != [0u8; 32]).then_some(&message_type_config.schema_hash);

            match self
                .schema_fetcher
                .fetch(&message_type_config.schema_uri, expected_hash)
                .await
            {
                Ok(schema) => {
                    tracing::debug!(
                        schema_uri = %message_type_config.schema_uri,
                        "Schema fetched successfully"
                    );
                    Some(schema)
                }
                Err(e) => {
                    tracing::warn!(
                        schema_uri = %message_type_config.schema_uri,
                        error = %e,
                        "Failed to fetch schema, proceeding without schema validation"
                    );
                    None
                }
            }
        };

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

    /// Validate a message as a Witness Validator.
    ///
    /// Witness validation skips stages 1 (replay) and 2 (nonce) since:
    /// - Replay: The primary validator already initialized the message
    /// - Nonce: The primary validator already consumed the nonce
    ///
    /// The witness independently verifies all other validation stages.
    pub async fn validate_witness(
        &self,
        message: &Message,
        bridge_client: &BridgeClient,
    ) -> Result<(), ValidationError> {
        // Fetch validation context from bridge
        let ctx = self.fetch_context(message, bridge_client).await?;

        // Stage 3: Timestamp freshness (witnesses still check this)
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

        // Skip Stage 9 (custom rules) for witnesses - rate limiting is per-validator

        tracing::info!(message_id = %hex::encode(message.id), "All witness validation stages passed");

        Ok(())
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
                selector: [0x55, 0x24, 0x10, 0x77],
                target: Address::ZERO,
                schema_hash: [0u8; 32],
                schema_uri: String::new(),
                enabled: true,
                updated_at: 0,
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
