# Validator Implementation Plan

> Implementation details for Message Passing Bridge validators.
> See `SPEC.md` for the specification.

## Overview

This document contains implementation details, code patterns, and examples for Primary and Witness validators. Validators are off-chain services that:

- Receive messages from applications (Primary)
- Validate messages against schemas and invariants
- Sign valid messages using EIP-712
- Publish messages to storage layers
- Submit signatures to the Bridge contract
- Monitor Bridge events and re-verify messages (Witness)

## 1. Validator Architecture

```
validator/
├── main.rs                    # Entry point, config loading
├── config.rs                  # Configuration structs
├── http/
│   ├── server.rs              # Axum HTTP server
│   ├── handlers.rs            # Request handlers
│   ├── middleware.rs          # Rate limiting, logging
│   └── auth.rs                # mTLS/API key authentication
├── validation/
│   ├── pipeline.rs            # Validation pipeline orchestration
│   ├── schema.rs              # JSON Schema validation
│   ├── calldata.rs            # ABI decoding/validation
│   ├── invariants/
│   │   ├── mod.rs
│   │   ├── on_chain.rs        # On-chain state invariants
│   │   ├── oracle.rs          # Oracle price invariants
│   │   └── app_logic.rs       # Application logic invariants
│   └── nonce.rs               # Nonce tracking
├── signing/
│   ├── eip712.rs              # EIP-712 typed data signing
│   ├── key_manager.rs         # Key loading/rotation
│   └── tee.rs                 # TEE key management
├── storage/
│   ├── publisher.rs           # Storage layer publication
│   ├── fetcher.rs             # Storage layer fetching
│   ├── arweave.rs
│   ├── ipfs.rs
│   └── gcs.rs
├── bridge/
│   ├── client.rs              # Bridge contract interaction
│   ├── events.rs              # Event monitoring
│   └── types.rs               # Contract types (alloy)
├── witness/
│   ├── discovery.rs           # Message discovery from storage
│   ├── rederivation.rs        # Metadata re-derivation
│   └── verification_api.rs    # External verification API client
├── oracle/
│   ├── chainlink.rs           # Chainlink price feeds
│   ├── pyth.rs                # Pyth price feeds
│   └── aggregator.rs          # Multi-oracle aggregation
├── rpc/
│   ├── client.rs              # Multi-chain RPC client
│   └── retry.rs               # Retry with backoff
└── metrics/
    ├── prometheus.rs          # Metrics export
    └── tracing.rs             # Distributed tracing
```

## 2. Error Types

```rust
#[derive(Debug, thiserror::Error)]
pub enum ValidationError {
    // Replay & Nonce
    #[error("REPLAY_DETECTED: message {0} already processed")]
    ReplayDetected(String),

    #[error("INVALID_NONCE: expected {expected}, got {provided} for domain {domain}")]
    InvalidNonce { domain: String, expected: u64, provided: u64 },

    // Timestamp
    #[error("TIMESTAMP_EXPIRED: message timestamp {timestamp} outside drift window")]
    TimestampExpired { timestamp: u64 },

    // Authorization
    #[error("APP_NOT_AUTHORIZED: domain {0} not registered")]
    AppNotAuthorized(String),

    #[error("MESSAGE_TYPE_NOT_REGISTERED: {0}")]
    MessageTypeNotRegistered(String),

    #[error("MESSAGE_TYPE_DISABLED: {0}")]
    MessageTypeDisabled(String),

    // Validation
    #[error("CALLDATA_INVALID: {0}")]
    CalldataInvalid(String),

    #[error("SCHEMA_VALIDATION_FAILED: {0}")]
    SchemaValidationFailed(String),

    // Invariants
    #[error("INVARIANT_VIOLATED: {message}")]
    InvariantViolated { invariant: String, message: String },

    #[error("INVARIANT_DATA_STALE: {source} data older than {max_age_seconds}s")]
    InvariantDataStale { source: String, max_age_seconds: u64 },

    #[error("INVARIANT_DATA_UNAVAILABLE: could not fetch {source}")]
    InvariantDataUnavailable { source: String },

    // Storage
    #[error("STORAGE_PUBLISH_FAILED: {0}")]
    StoragePublishFailed(String),

    #[error("STORAGE_FETCH_FAILED: {0}")]
    StorageFetchFailed(String),

    // Bridge
    #[error("BRIDGE_SUBMIT_FAILED: {0}")]
    BridgeSubmitFailed(String),

    // Internal
    #[error("INTERNAL_ERROR: {0}")]
    InternalError(String),
}

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub code: String,
    pub message: String,
    pub details: Option<serde_json::Value>,
}

impl From<ValidationError> for ErrorResponse {
    fn from(err: ValidationError) -> Self {
        let code = match &err {
            ValidationError::ReplayDetected(_) => "REPLAY_DETECTED",
            ValidationError::InvalidNonce { .. } => "INVALID_NONCE",
            ValidationError::TimestampExpired { .. } => "TIMESTAMP_EXPIRED",
            ValidationError::AppNotAuthorized(_) => "APP_NOT_AUTHORIZED",
            ValidationError::MessageTypeNotRegistered(_) => "MESSAGE_TYPE_NOT_REGISTERED",
            ValidationError::MessageTypeDisabled(_) => "MESSAGE_TYPE_DISABLED",
            ValidationError::CalldataInvalid(_) => "CALLDATA_INVALID",
            ValidationError::SchemaValidationFailed(_) => "SCHEMA_VALIDATION_FAILED",
            ValidationError::InvariantViolated { .. } => "INVARIANT_VIOLATED",
            ValidationError::InvariantDataStale { .. } => "INVARIANT_DATA_STALE",
            ValidationError::InvariantDataUnavailable { .. } => "INVARIANT_DATA_UNAVAILABLE",
            ValidationError::StoragePublishFailed(_) => "STORAGE_PUBLISH_FAILED",
            ValidationError::StorageFetchFailed(_) => "STORAGE_FETCH_FAILED",
            ValidationError::BridgeSubmitFailed(_) => "BRIDGE_SUBMIT_FAILED",
            ValidationError::InternalError(_) => "INTERNAL_ERROR",
        };

        ErrorResponse {
            code: code.to_string(),
            message: err.to_string(),
            details: None,
        }
    }
}
```

## 3. Validation Pipeline

### 3.1 Pipeline Stages

```rust
pub struct ValidationPipeline {
    nonce_tracker: NonceTracker,
    schema_cache: SchemaCache,
    rpc_client: RpcClient,
    oracle_client: OracleClient,
    config: ValidationConfig,
}

impl ValidationPipeline {
    pub async fn validate(&self, message: &Message) -> Result<ValidationResult, ValidationError> {
        // Stage 1: Replay protection
        let message_id = compute_message_id(message);
        if self.is_processed(&message_id).await? {
            return Err(ValidationError::ReplayDetected(message_id.to_string()));
        }

        // Stage 2: Nonce check
        let expected = self.nonce_tracker.get_expected(message.domain).await?;
        if message.nonce != expected {
            return Err(ValidationError::InvalidNonce {
                domain: hex::encode(message.domain),
                expected,
                provided: message.nonce,
            });
        }

        // Stage 3: Timestamp freshness
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        let drift = (now as i64 - message.timestamp as i64).abs() as u64;
        if drift > self.config.max_clock_drift_seconds {
            return Err(ValidationError::TimestampExpired {
                timestamp: message.timestamp,
            });
        }

        // Stage 4: Application authorization
        let app_config = self.get_app_config(message.domain).await?
            .ok_or_else(|| ValidationError::AppNotAuthorized(hex::encode(message.domain)))?;

        // Stage 5: Message type validation
        let type_config = self.get_message_type_config(&message.message_type).await?
            .ok_or_else(|| ValidationError::MessageTypeNotRegistered(message.message_type.clone()))?;

        if !type_config.enabled {
            return Err(ValidationError::MessageTypeDisabled(message.message_type.clone()));
        }

        // Stage 6: Calldata validation
        self.validate_calldata(&message.calldata, &message.message_type)?;

        // Stage 7: Schema validation
        let schema = self.schema_cache.get_or_fetch(&type_config.schema_uri).await?;
        self.validate_schema(&message.metadata, &schema)?;

        // Stage 8: Invariant checks
        self.check_invariants(message, &type_config).await?;

        // Stage 9: Custom rules (rate limits, thresholds)
        self.check_custom_rules(message, &app_config).await?;

        Ok(ValidationResult::success(message_id))
    }
}
```

### 3.2 Message ID Computation

```rust
pub fn compute_message_id(message: &Message) -> [u8; 32] {
    let mut hasher = Keccak256::new();

    // Hash components in order matching Bridge contract
    hasher.update(message.message_type.as_bytes());
    hasher.update(&keccak256(&message.calldata));
    hasher.update(&message.metadata_hash);
    hasher.update(&message.nonce.to_be_bytes());
    hasher.update(&message.timestamp.to_be_bytes());
    hasher.update(&message.domain);

    hasher.finalize().into()
}

pub fn compute_metadata_hash(metadata: &serde_json::Value) -> [u8; 32] {
    // RFC 8785 JSON Canonicalization
    let canonical = json_canonicalize(metadata);
    keccak256(canonical.as_bytes())
}
```

### 3.3 Nonce Tracking

```rust
pub struct NonceTracker {
    db: Database,  // Persistent storage (SQLite, RocksDB, etc.)
}

impl NonceTracker {
    pub async fn get_expected(&self, domain: [u8; 32]) -> Result<u64, Error> {
        let last = self.db.get_last_nonce(domain).await?.unwrap_or(0);
        Ok(last + 1)
    }

    pub async fn consume(&self, domain: [u8; 32], nonce: u64) -> Result<(), Error> {
        // Called on both initialize AND reject
        self.db.set_last_nonce(domain, nonce).await
    }

    pub async fn sync_from_bridge(&self, bridge: &BridgeClient) -> Result<(), Error> {
        // Sync nonces from on-chain state on startup
        for domain in self.db.get_all_domains().await? {
            let on_chain_nonce = bridge.get_last_nonce(domain).await?;
            let local_nonce = self.db.get_last_nonce(domain).await?.unwrap_or(0);

            if on_chain_nonce > local_nonce {
                tracing::warn!(
                    domain = %hex::encode(domain),
                    local = local_nonce,
                    on_chain = on_chain_nonce,
                    "Syncing nonce from chain"
                );
                self.db.set_last_nonce(domain, on_chain_nonce).await?;
            }
        }
        Ok(())
    }
}
```

## 4. Invariant Checking

### 4.1 Invariant Registry

```rust
pub struct InvariantRegistry {
    on_chain: Vec<Box<dyn OnChainInvariant>>,
    oracle: Vec<Box<dyn OracleInvariant>>,
    app_logic: Vec<Box<dyn AppLogicInvariant>>,
}

impl InvariantRegistry {
    pub async fn check_all(&self, message: &Message, ctx: &InvariantContext) -> Result<(), ValidationError> {
        // Run all invariant checks in parallel where possible
        let on_chain_results = futures::future::try_join_all(
            self.on_chain.iter().map(|inv| inv.check(message, ctx))
        );

        let oracle_results = futures::future::try_join_all(
            self.oracle.iter().map(|inv| inv.check(message, ctx))
        );

        let app_results = futures::future::try_join_all(
            self.app_logic.iter().map(|inv| inv.check(message, ctx))
        );

        // Wait for all checks
        let (on_chain, oracle, app) = tokio::try_join!(
            on_chain_results,
            oracle_results,
            app_results
        )?;

        Ok(())
    }
}
```

### 4.2 On-Chain State Invariants

```rust
#[async_trait]
pub trait OnChainInvariant: Send + Sync {
    fn name(&self) -> &str;
    async fn check(&self, message: &Message, ctx: &InvariantContext) -> Result<(), ValidationError>;
}

pub struct SupplyCapInvariant {
    rpc: RpcClient,
}

#[async_trait]
impl OnChainInvariant for SupplyCapInvariant {
    fn name(&self) -> &str { "supply_cap" }

    async fn check(&self, message: &Message, ctx: &InvariantContext) -> Result<(), ValidationError> {
        // Only applies to mint operations
        if !message.message_type.starts_with("mint") {
            return Ok(());
        }

        // Extract mint amount from calldata
        let amount = decode_mint_amount(&message.calldata)?;

        // Get current supply from target contract
        let target = ctx.type_config.target;
        let current_supply: U256 = self.rpc.call(target, "totalSupply()").await?;

        // Get max supply from metadata
        let max_supply = message.metadata["maxSupply"]
            .as_str()
            .and_then(|s| U256::from_dec_str(s).ok())
            .ok_or_else(|| ValidationError::SchemaValidationFailed("missing maxSupply".into()))?;

        // Check invariant
        if current_supply + amount > max_supply {
            return Err(ValidationError::InvariantViolated {
                invariant: self.name().to_string(),
                message: format!(
                    "Supply cap exceeded: {} + {} > {}",
                    current_supply, amount, max_supply
                ),
            });
        }

        Ok(())
    }
}
```

### 4.3 Oracle Price Invariants

```rust
pub struct PriceDeviationInvariant {
    oracle: OracleClient,
    max_age_seconds: u64,
}

#[async_trait]
impl OracleInvariant for PriceDeviationInvariant {
    fn name(&self) -> &str { "price_deviation" }

    async fn check(&self, message: &Message, ctx: &InvariantContext) -> Result<(), ValidationError> {
        // Only applies if metadata contains exchange rate
        let Some(app_rate) = message.metadata.get("exchangeRate") else {
            return Ok(());
        };

        let app_rate: Decimal = app_rate.as_str()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| ValidationError::SchemaValidationFailed("invalid exchangeRate".into()))?;

        let max_deviation: Decimal = message.metadata
            .get("maxDeviation")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok())
            .unwrap_or_else(|| Decimal::new(5, 0)); // Default 5%

        // Fetch oracle price
        let from_token = message.metadata["fromToken"].as_str().unwrap();
        let to_token = message.metadata["toToken"].as_str().unwrap();

        let oracle_price = self.oracle
            .get_price(from_token, to_token)
            .await
            .map_err(|e| ValidationError::InvariantDataUnavailable {
                source: format!("oracle:{}/{}", from_token, to_token),
            })?;

        // Check freshness
        let age = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_secs() - oracle_price.timestamp;

        if age > self.max_age_seconds {
            return Err(ValidationError::InvariantDataStale {
                source: "chainlink".to_string(),
                max_age_seconds: self.max_age_seconds,
            });
        }

        // Check deviation
        let deviation_pct = ((app_rate - oracle_price.value).abs() / oracle_price.value) * Decimal::new(100, 0);

        if deviation_pct > max_deviation {
            return Err(ValidationError::InvariantViolated {
                invariant: self.name().to_string(),
                message: format!(
                    "Price deviation {:.2}% exceeds max {:.2}%",
                    deviation_pct, max_deviation
                ),
            });
        }

        Ok(())
    }
}
```

### 4.4 Application Logic Invariants

```rust
pub struct GameInvariant;

#[async_trait]
impl AppLogicInvariant for GameInvariant {
    fn name(&self) -> &str { "game_rules" }

    async fn check(&self, message: &Message, _ctx: &InvariantContext) -> Result<(), ValidationError> {
        let metadata = &message.metadata;

        // Only applies to game-related message types
        if !message.message_type.contains("game") {
            return Ok(());
        }

        // Invariant 1: Only one winner per game
        if let Some(winner_count) = metadata.get("gameWinnerCount") {
            if winner_count.as_u64() != Some(0) {
                return Err(ValidationError::InvariantViolated {
                    invariant: self.name().to_string(),
                    message: format!(
                        "Game {} already has a winner",
                        metadata["gameId"]
                    ),
                });
            }
        }

        // Invariant 2: Minimum play interval
        if let (Some(current), Some(last), Some(min_interval)) = (
            metadata.get("currentTimestamp").and_then(|v| v.as_u64()),
            metadata.get("playerLastPlayTimestamp").and_then(|v| v.as_u64()),
            metadata.get("minPlayInterval").and_then(|v| v.as_u64()),
        ) {
            let elapsed = current.saturating_sub(last);
            if elapsed < min_interval {
                return Err(ValidationError::InvariantViolated {
                    invariant: self.name().to_string(),
                    message: format!(
                        "Player must wait {}s ({}s remaining)",
                        min_interval, min_interval - elapsed
                    ),
                });
            }
        }

        Ok(())
    }
}
```

## 5. Witness Validator

### 5.1 Event Monitoring

```rust
pub struct WitnessEventMonitor {
    bridge: BridgeClient,
    storage: StorageClient,
    validator: WitnessValidator,
}

impl WitnessEventMonitor {
    pub async fn run(&self, shutdown: watch::Receiver<bool>) -> Result<(), Error> {
        let mut event_stream = self.bridge.subscribe_events("MessageInitialized").await?;

        loop {
            tokio::select! {
                Some(event) = event_stream.next() => {
                    if let Err(e) = self.handle_event(event).await {
                        tracing::error!(error = %e, "Failed to handle event");
                        // Continue processing other events
                    }
                }
                _ = shutdown.changed() => {
                    tracing::info!("Shutting down event monitor");
                    break;
                }
            }
        }

        Ok(())
    }

    async fn handle_event(&self, event: MessageInitializedEvent) -> Result<(), Error> {
        let message_id = event.message_id;
        let storage_ref = event.storage_ref;

        tracing::info!(
            message_id = %hex::encode(message_id),
            storage_ref = %storage_ref,
            "Discovered new message"
        );

        // Fetch message from storage
        let message = self.storage.fetch(&storage_ref).await?;

        // Verify message ID matches
        let computed_id = compute_message_id(&message);
        if computed_id != message_id {
            tracing::error!(
                expected = %hex::encode(message_id),
                computed = %hex::encode(computed_id),
                "Message ID mismatch - possible tampering"
            );
            return Err(Error::MessageIdMismatch);
        }

        // Validate and sign if valid
        self.validator.validate_and_sign(message).await
    }
}
```

### 5.2 Metadata Re-Derivation

```rust
pub struct MetadataRederivation {
    rpc_clients: HashMap<String, RpcClient>,  // chain_id -> client
    oracle: OracleClient,
    tolerance: RederivationTolerance,
}

impl MetadataRederivation {
    pub async fn verify(&self, message: &Message) -> Result<(), ValidationError> {
        let metadata = &message.metadata;

        // Verify source transaction if provided
        if let Some(tx_hash) = metadata.get("sourceTxHash").and_then(|v| v.as_str()) {
            self.verify_source_tx(tx_hash, metadata).await?;
        }

        // Verify exchange rate against oracle
        if metadata.contains_key("exchangeRate") {
            self.verify_exchange_rate(metadata).await?;
        }

        // Verify on-chain state claims
        if metadata.contains_key("currentTotalSupply") {
            self.verify_supply(metadata).await?;
        }

        Ok(())
    }

    async fn verify_source_tx(&self, tx_hash: &str, metadata: &serde_json::Value) -> Result<(), ValidationError> {
        let chain = metadata["sourceChain"].as_str().unwrap();
        let rpc = self.rpc_clients.get(chain)
            .ok_or_else(|| ValidationError::InvariantDataUnavailable {
                source: format!("rpc:{}", chain),
            })?;

        let tx = rpc.get_transaction(tx_hash).await
            .map_err(|_| ValidationError::InvariantDataUnavailable {
                source: format!("tx:{}", tx_hash),
            })?;

        // Verify deposit amount from event logs
        if let Some(claimed_amount) = metadata.get("depositAmount") {
            let actual = extract_deposit_amount_from_logs(&tx.logs)?;
            let claimed: U256 = claimed_amount.as_str()
                .and_then(|s| U256::from_dec_str(s).ok())
                .unwrap();

            if actual != claimed {
                return Err(ValidationError::InvariantViolated {
                    invariant: "deposit_amount".to_string(),
                    message: format!("Deposit mismatch: actual {} != claimed {}", actual, claimed),
                });
            }
        }

        Ok(())
    }

    async fn verify_exchange_rate(&self, metadata: &serde_json::Value) -> Result<(), ValidationError> {
        let from_token = metadata["fromToken"].as_str().unwrap();
        let to_token = metadata["toToken"].as_str().unwrap();
        let claimed_rate: Decimal = metadata["exchangeRate"]
            .as_str()
            .and_then(|s| s.parse().ok())
            .unwrap();

        let oracle_price = self.oracle.get_price(from_token, to_token).await?;
        let deviation = ((claimed_rate - oracle_price.value).abs() / oracle_price.value) * Decimal::new(100, 0);

        if deviation > self.tolerance.price_deviation_pct {
            return Err(ValidationError::InvariantViolated {
                invariant: "exchange_rate".to_string(),
                message: format!("Rate deviation {:.2}% exceeds tolerance", deviation),
            });
        }

        Ok(())
    }
}
```

### 5.3 Verification API Client

```rust
pub struct VerificationApiClient {
    http: reqwest::Client,
    timeout: Duration,
    reject_on_timeout: bool,
}

impl VerificationApiClient {
    pub async fn verify(&self, message: &Message) -> Result<(), ValidationError> {
        let Some(api_url) = message.metadata.get("verificationApiUrl").and_then(|v| v.as_str()) else {
            // No verification API configured - trust Primary
            tracing::debug!("No verificationApiUrl, trusting Primary validation");
            return Ok(());
        };

        let response = self.http
            .post(format!("{}/verify", api_url))
            .json(&serde_json::json!({
                "messageId": hex::encode(compute_message_id(message)),
                "metadata": message.metadata
            }))
            .timeout(self.timeout)
            .send()
            .await;

        match response {
            Ok(resp) => {
                let result: VerificationResponse = resp.json().await
                    .map_err(|e| ValidationError::InternalError(e.to_string()))?;

                if !result.valid {
                    return Err(ValidationError::InvariantViolated {
                        invariant: "verification_api".to_string(),
                        message: result.reason.unwrap_or_else(|| "Rejected by verification API".into()),
                    });
                }

                Ok(())
            }
            Err(e) if e.is_timeout() => {
                if self.reject_on_timeout {
                    Err(ValidationError::InvariantDataUnavailable {
                        source: "verification_api".to_string(),
                    })
                } else {
                    tracing::warn!("Verification API timeout, trusting Primary");
                    Ok(())
                }
            }
            Err(e) => Err(ValidationError::InvariantDataUnavailable {
                source: format!("verification_api: {}", e),
            }),
        }
    }
}

#[derive(Deserialize)]
struct VerificationResponse {
    valid: bool,
    reason: Option<String>,
}
```

## 6. Rejection Flow

### 6.1 Primary Validator Rejection

```rust
impl PrimaryValidator {
    pub async fn handle_message(&self, request: MessageRequest) -> Result<MessageResponse, ValidationError> {
        let message = Message::from_request(request)?;
        let message_id = compute_message_id(&message);

        match self.pipeline.validate(&message).await {
            Ok(result) => {
                // Sign and publish
                let signature = self.sign(&message)?;
                let storage_ref = self.storage.publish(&message, &signature).await?;
                self.bridge.initialize_and_sign(message_id, &message, &signature, &storage_ref).await?;

                Ok(MessageResponse::accepted(message_id, signature, storage_ref))
            }
            Err(validation_error) => {
                // Reject on-chain to consume nonce
                self.reject_proposal(&message, &message_id, &validation_error).await?;

                Err(validation_error)
            }
        }
    }

    async fn reject_proposal(
        &self,
        message: &Message,
        message_id: &[u8; 32],
        error: &ValidationError,
    ) -> Result<(), Error> {
        // Compute reason hash
        let reason = ErrorResponse::from(error.clone());
        let reason_json = serde_json::to_string(&reason)?;
        let reason_hash = keccak256(reason_json.as_bytes());

        // Publish rejection reason to storage
        let reason_ref = self.storage.publish_rejection(&reason_json).await?;

        // Submit rejection to Bridge (consumes nonce)
        self.bridge.reject_proposal(
            message_id,
            &message.message_type,
            &message.domain,
            message.nonce,
            &reason_hash,
            &reason_ref,
        ).await?;

        // Track locally
        self.nonce_tracker.consume(message.domain, message.nonce).await?;

        tracing::info!(
            message_id = %hex::encode(message_id),
            error_code = %reason.code,
            "Proposal rejected on-chain"
        );

        Ok(())
    }
}
```

### 6.2 Witness Validator Rejection

```rust
impl WitnessValidator {
    pub async fn validate_and_sign(&self, message: Message) -> Result<(), Error> {
        let message_id = compute_message_id(&message);

        // Re-derive and verify metadata
        if let Err(e) = self.rederivation.verify(&message).await {
            self.reject_message(&message_id, &e).await?;
            return Err(e.into());
        }

        // Check invariants independently
        if let Err(e) = self.invariants.check_all(&message, &self.ctx).await {
            self.reject_message(&message_id, &e).await?;
            return Err(e.into());
        }

        // Optional: Call verification API
        if let Err(e) = self.verification_api.verify(&message).await {
            self.reject_message(&message_id, &e).await?;
            return Err(e.into());
        }

        // Sign and submit
        let signature = self.sign(&message)?;
        self.bridge.sign_message(&message_id, &signature).await?;

        tracing::info!(
            message_id = %hex::encode(message_id),
            "Message signed by witness"
        );

        Ok(())
    }

    async fn reject_message(
        &self,
        message_id: &[u8; 32],
        error: &ValidationError,
    ) -> Result<(), Error> {
        let reason = ErrorResponse::from(error.clone());
        let reason_json = serde_json::to_string(&reason)?;
        let reason_hash = keccak256(reason_json.as_bytes());

        // Publish rejection reason
        let reason_ref = self.storage.publish_rejection(&reason_json).await?;

        // Submit rejection to Bridge (informational - doesn't block threshold)
        self.bridge.reject_message(message_id, &reason_hash, &reason_ref).await?;

        tracing::warn!(
            message_id = %hex::encode(message_id),
            error_code = %reason.code,
            "Message rejected by witness"
        );

        Ok(())
    }
}
```

## 7. Bridge Client

### 7.1 Contract Interaction

```rust
use alloy::{
    contract::SolCall,
    network::EthereumWallet,
    primitives::{Address, Bytes, U256},
    providers::ProviderBuilder,
    signers::local::PrivateKeySigner,
};

pub struct BridgeClient {
    provider: Provider,
    wallet: EthereumWallet,
    bridge_address: Address,
}

impl BridgeClient {
    pub async fn initialize_and_sign(
        &self,
        message_id: [u8; 32],
        message: &Message,
        signature: &[u8],
        storage_ref: &str,
    ) -> Result<TxHash, Error> {
        let call = IMessageBridge::initializeAndSignCall {
            messageId: message_id.into(),
            messageType: message.message_type.clone(),
            calldata_: message.calldata.clone().into(),
            metadataHash: message.metadata_hash.into(),
            storageRef: storage_ref.to_string(),
            nonce: message.nonce,
            timestamp: message.timestamp,
            domain: message.domain.into(),
            signature: signature.to_vec().into(),
        };

        let tx = self.provider
            .send_transaction(
                TransactionRequest::default()
                    .to(self.bridge_address)
                    .input(call.abi_encode().into())
                    .value(U256::from(message.value.unwrap_or(0)))
            )
            .await?;

        let receipt = tx.get_receipt().await?;
        Ok(receipt.transaction_hash)
    }

    pub async fn sign_message(
        &self,
        message_id: &[u8; 32],
        signature: &[u8],
    ) -> Result<TxHash, Error> {
        let call = IMessageBridge::signMessageCall {
            messageId: (*message_id).into(),
            signature: signature.to_vec().into(),
        };

        let tx = self.provider
            .send_transaction(
                TransactionRequest::default()
                    .to(self.bridge_address)
                    .input(call.abi_encode().into())
            )
            .await?;

        let receipt = tx.get_receipt().await?;
        Ok(receipt.transaction_hash)
    }

    pub async fn reject_proposal(
        &self,
        message_id: &[u8; 32],
        message_type: &str,
        domain: &[u8; 32],
        nonce: u64,
        reason_hash: &[u8; 32],
        reason_ref: &str,
    ) -> Result<TxHash, Error> {
        let call = IMessageBridge::rejectProposalCall {
            messageId: (*message_id).into(),
            messageType: message_type.to_string(),
            domain: (*domain).into(),
            nonce,
            reasonHash: (*reason_hash).into(),
            reasonRef: reason_ref.to_string(),
        };

        let tx = self.provider
            .send_transaction(
                TransactionRequest::default()
                    .to(self.bridge_address)
                    .input(call.abi_encode().into())
            )
            .await?;

        let receipt = tx.get_receipt().await?;
        Ok(receipt.transaction_hash)
    }

    pub async fn get_last_nonce(&self, domain: [u8; 32]) -> Result<u64, Error> {
        let call = IMessageBridge::getLastNonceCall {
            domain: domain.into(),
        };

        let result = self.provider
            .call(&TransactionRequest::default()
                .to(self.bridge_address)
                .input(call.abi_encode().into()))
            .await?;

        Ok(u64::from_be_bytes(result[24..32].try_into()?))
    }

    pub async fn subscribe_events(&self, event_name: &str) -> Result<EventStream, Error> {
        let filter = Filter::new()
            .address(self.bridge_address)
            .event(event_name);

        Ok(self.provider.subscribe_logs(&filter).await?)
    }
}
```

## 8. HTTP API

### 8.1 Endpoints

```yaml
# Primary Validator API

POST /messages
  Description: Submit message for validation and signing
  Auth: mTLS or API key
  Request:
    messageType: string          # e.g., "mint(address,uint256)"
    calldata: hex string         # ABI-encoded function parameters
    metadata: object             # Evidence for validators
    nonce: uint64                # Sequential per domain
    timestamp: uint64            # Unix timestamp
    domain: hex string           # 32-byte application ID
    value?: string               # Optional: wei amount for payable calls
  Response (200):
    status: "accepted"
    messageId: hex string
    signature: hex string
    storageRef: string
  Response (400):
    status: "rejected"
    error:
      code: string
      message: string
      details?: object

GET /messages/{messageId}
  Description: Get message status
  Response:
    id: hex string
    status: "pending" | "signed" | "published" | "submitted" | "executed" | "failed" | "expired"
    stage: "Pending" | "Ready" | "PreExecution" | "Executing" | "PostExecution" | "Completed" | "Failed" | "Expired"
    signaturesCollected: number
    rejectionsCollected: number
    storageRef?: string
    bridgeTxHash?: hex string
    executionTxHash?: hex string

GET /health
  Response:
    healthy: boolean
    mode: "primary" | "witness"
    synced: boolean
    bridgeConnection: boolean
    storageConnection: boolean
    oracleConnection: boolean
    lastProcessedBlock: number
    lastProcessedNonce: map[domain => nonce]

GET /schemas/{messageType}
  Response:
    messageType: string
    schema: object
    schemaHash: hex string
    schemaUri: string
    source: "cache" | "chain" | "storage"
    cachedAt: timestamp

GET /metrics
  Description: Prometheus metrics
  Response: text/plain (Prometheus format)
```

### 8.2 Request/Response Types

```rust
#[derive(Debug, Deserialize)]
pub struct MessageRequest {
    #[serde(rename = "messageType")]
    pub message_type: String,
    pub calldata: String,  // hex-encoded
    pub metadata: serde_json::Value,
    pub nonce: u64,
    pub timestamp: u64,
    pub domain: String,  // hex-encoded bytes32
    pub value: Option<String>,  // wei as decimal string
}

#[derive(Debug, Serialize)]
#[serde(tag = "status")]
pub enum MessageResponse {
    #[serde(rename = "accepted")]
    Accepted {
        #[serde(rename = "messageId")]
        message_id: String,
        signature: String,
        #[serde(rename = "storageRef")]
        storage_ref: String,
    },
    #[serde(rename = "rejected")]
    Rejected {
        error: ErrorResponse,
    },
}

#[derive(Debug, Serialize)]
pub struct MessageStatusResponse {
    pub id: String,
    pub status: String,
    pub stage: String,
    #[serde(rename = "signaturesCollected")]
    pub signatures_collected: u64,
    #[serde(rename = "rejectionsCollected")]
    pub rejections_collected: u64,
    #[serde(rename = "storageRef", skip_serializing_if = "Option::is_none")]
    pub storage_ref: Option<String>,
    #[serde(rename = "bridgeTxHash", skip_serializing_if = "Option::is_none")]
    pub bridge_tx_hash: Option<String>,
}
```

## 9. Storage Publication

### 9.1 Storage Record Format

```rust
#[derive(Debug, Serialize, Deserialize)]
pub struct StorageRecord {
    pub message: StoredMessage,
    #[serde(rename = "primarySignature")]
    pub primary_signature: SignatureRecord,
    pub publication: PublicationRecord,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StoredMessage {
    pub id: String,  // hex-encoded bytes32
    #[serde(rename = "messageType")]
    pub message_type: String,
    pub calldata: String,  // hex-encoded
    pub metadata: serde_json::Value,
    #[serde(rename = "metadataHash")]
    pub metadata_hash: String,  // hex-encoded bytes32
    pub nonce: u64,
    pub timestamp: u64,
    pub domain: String,  // hex-encoded bytes32
    pub value: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SignatureRecord {
    pub validator: String,  // Ethereum address
    pub signature: String,  // hex-encoded
    #[serde(rename = "signedAt")]
    pub signed_at: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PublicationRecord {
    #[serde(rename = "publishedBy")]
    pub published_by: String,
    #[serde(rename = "publishedAt")]
    pub published_at: u64,
}
```

### 9.2 Multi-Layer Publisher

```rust
pub struct StoragePublisher {
    arweave: Option<ArweaveClient>,
    ipfs: Option<IpfsClient>,
    gcs: Option<GcsClient>,
}

impl StoragePublisher {
    pub async fn publish(&self, message: &Message, signature: &[u8]) -> Result<String, Error> {
        let record = StorageRecord {
            message: StoredMessage::from(message),
            primary_signature: SignatureRecord {
                validator: self.address.to_string(),
                signature: hex::encode(signature),
                signed_at: now_unix(),
            },
            publication: PublicationRecord {
                published_by: self.address.to_string(),
                published_at: now_unix(),
            },
        };

        let json = serde_json::to_string_pretty(&record)?;
        let mut refs = Vec::new();

        // Publish to all configured layers in parallel
        let mut futures = Vec::new();

        if let Some(arweave) = &self.arweave {
            futures.push(async move {
                let tx_id = arweave.publish(&json).await?;
                Ok::<_, Error>(format!("ar://{}", tx_id))
            }.boxed());
        }

        if let Some(ipfs) = &self.ipfs {
            futures.push(async move {
                let cid = ipfs.publish(&json).await?;
                Ok::<_, Error>(format!("ipfs://{}", cid))
            }.boxed());
        }

        if let Some(gcs) = &self.gcs {
            let message_id = hex::encode(compute_message_id(message));
            futures.push(async move {
                let path = gcs.publish(&json, &message_id).await?;
                Ok::<_, Error>(format!("gcs://{}", path))
            }.boxed());
        }

        let results = futures::future::join_all(futures).await;

        for result in results {
            match result {
                Ok(ref_uri) => refs.push(ref_uri),
                Err(e) => tracing::warn!(error = %e, "Failed to publish to storage layer"),
            }
        }

        if refs.is_empty() {
            return Err(Error::StoragePublishFailed("All storage layers failed".into()));
        }

        Ok(refs.join("|"))
    }
}
```

## 10. EIP-712 Signing

```rust
use alloy::sol_types::{eip712_domain, SolStruct};

sol! {
    #[derive(Debug)]
    struct MessageData {
        bytes32 messageId;
        string messageType;
        bytes calldata_;
        bytes32 metadataHash;
        uint64 nonce;
        uint64 timestamp;
        bytes32 domain;
    }
}

pub struct EIP712Signer {
    domain: alloy::sol_types::Eip712Domain,
    signer: PrivateKeySigner,
}

impl EIP712Signer {
    pub fn new(chain_id: u64, bridge_address: Address, private_key: &[u8]) -> Result<Self, Error> {
        let domain = eip712_domain! {
            name: "SyndBridge",
            version: "1",
            chain_id: chain_id,
            verifying_contract: bridge_address,
        };

        let signer = PrivateKeySigner::from_bytes(&B256::from_slice(private_key))?;

        Ok(Self { domain, signer })
    }

    pub fn sign(&self, message: &Message) -> Result<Vec<u8>, Error> {
        let data = MessageData {
            messageId: compute_message_id(message).into(),
            messageType: message.message_type.clone(),
            calldata_: message.calldata.clone().into(),
            metadataHash: message.metadata_hash.into(),
            nonce: message.nonce,
            timestamp: message.timestamp,
            domain: message.domain.into(),
        };

        let hash = data.eip712_signing_hash(&self.domain);
        let signature = self.signer.sign_hash_sync(&hash)?;

        Ok(signature.as_bytes().to_vec())
    }

    pub fn address(&self) -> Address {
        self.signer.address()
    }
}
```

## 11. TEE Key Management

### 11.1 Key Generation in TEE

```rust
pub struct TeeKeyManager {
    enclave: EnclaveClient,
}

impl TeeKeyManager {
    pub async fn generate_signing_key(&self) -> Result<TeeKey, Error> {
        // Request key generation inside enclave
        let key_id = self.enclave.generate_secp256k1_key().await?;

        // Get public key (private key never leaves enclave)
        let public_key = self.enclave.get_public_key(key_id).await?;

        // Derive Ethereum address
        let address = public_key_to_address(&public_key);

        // Generate attestation binding key to enclave
        let attestation = self.enclave.generate_attestation(key_id).await?;

        Ok(TeeKey {
            key_id,
            address,
            attestation,
        })
    }

    pub async fn sign(&self, key_id: &str, digest: &[u8; 32]) -> Result<Vec<u8>, Error> {
        // Signing happens inside enclave
        self.enclave.sign(key_id, digest).await
    }
}

pub struct TeeKey {
    pub key_id: String,
    pub address: Address,
    pub attestation: Vec<u8>,
}
```

### 11.2 Attestation Verification

```rust
pub fn verify_attestation(attestation: &[u8], expected_address: Address) -> Result<AttestationInfo, Error> {
    // Parse attestation token (format depends on TEE provider)
    let token = parse_attestation_token(attestation)?;

    // Verify signature from TEE platform
    verify_platform_signature(&token)?;

    // Extract and verify claims
    let claims = extract_claims(&token)?;

    // Verify key fingerprint matches expected address
    let key_fingerprint = claims.get("key_fingerprint")
        .ok_or(Error::InvalidAttestation("missing key_fingerprint"))?;

    let derived_address = fingerprint_to_address(key_fingerprint)?;
    if derived_address != expected_address {
        return Err(Error::InvalidAttestation("address mismatch"));
    }

    Ok(AttestationInfo {
        code_hash: claims["code_hash"].clone(),
        version: claims["version"].clone(),
        timestamp: claims["timestamp"].parse()?,
    })
}

pub struct AttestationInfo {
    pub code_hash: String,
    pub version: String,
    pub timestamp: u64,
}
```

## 12. Configuration

```toml
[validator]
mode = "primary"  # or "witness"
address = "0x..."  # Validator Ethereum address

[keys]
# Option 1: Local file (development only)
private_key_path = "/secrets/signing_key"

# Option 2: TEE-managed (production)
tee_provider = "gcp-confidential-space"  # or "aws-nitro"
tee_key_id = "validator-signing-key"

[bridge]
address = "0x..."
rpc_url = "https://eth-mainnet.g.alchemy.com/v2/..."
chain_id = 1
confirmation_blocks = 2

[storage]
# At least one required
[storage.arweave]
enabled = true
gateway = "https://arweave.net"
wallet_path = "/secrets/arweave_wallet.json"

[storage.ipfs]
enabled = true
api_url = "http://localhost:5001"
pin = true

[storage.gcs]
enabled = false
bucket = "synd-bridge-messages"
credentials_path = "/secrets/gcs_credentials.json"

[http]
bind = "0.0.0.0:8080"
tls_cert = "/certs/server.crt"
tls_key = "/certs/server.key"
client_ca = "/certs/ca.crt"  # For mTLS
rate_limit_per_second = 100

[oracle]
[oracle.chainlink]
rpc_url = "https://..."
feeds = ["ETH/USD", "BTC/USD", "USDC/USD"]

[oracle.pyth]
endpoint = "https://hermes.pyth.network"
feeds = ["ETH/USD", "BTC/USD"]

max_age_seconds = 300

[validation]
max_clock_drift_seconds = 60
reject_on_verification_timeout = false
schema_cache_ttl_seconds = 3600

[witness]
# Witness-specific settings
poll_interval_ms = 1000
rederivation_tolerance_pct = 5.0

[metrics]
enabled = true
bind = "0.0.0.0:9090"

[logging]
level = "info"
format = "json"
```

## 13. Metrics & Observability

```rust
use prometheus::{Counter, Histogram, IntGauge};

lazy_static! {
    // Counters
    static ref MESSAGES_RECEIVED: Counter = register_counter!(
        "validator_messages_received_total",
        "Total messages received"
    ).unwrap();

    static ref MESSAGES_ACCEPTED: Counter = register_counter!(
        "validator_messages_accepted_total",
        "Total messages accepted and signed"
    ).unwrap();

    static ref MESSAGES_REJECTED: Counter = register_counter!(
        "validator_messages_rejected_total",
        "Total messages rejected"
    ).unwrap();

    static ref VALIDATION_ERRORS: Counter = register_counter_vec!(
        "validator_validation_errors_total",
        "Validation errors by type",
        &["error_code"]
    ).unwrap();

    // Histograms
    static ref VALIDATION_DURATION: Histogram = register_histogram!(
        "validator_validation_duration_seconds",
        "Time spent validating messages"
    ).unwrap();

    static ref STORAGE_PUBLISH_DURATION: Histogram = register_histogram!(
        "validator_storage_publish_duration_seconds",
        "Time spent publishing to storage"
    ).unwrap();

    static ref BRIDGE_TX_DURATION: Histogram = register_histogram!(
        "validator_bridge_tx_duration_seconds",
        "Time spent submitting to bridge"
    ).unwrap();

    // Gauges
    static ref LAST_PROCESSED_NONCE: IntGauge = register_int_gauge_vec!(
        "validator_last_processed_nonce",
        "Last processed nonce by domain",
        &["domain"]
    ).unwrap();

    static ref PENDING_MESSAGES: IntGauge = register_int_gauge!(
        "validator_pending_messages",
        "Number of messages pending processing"
    ).unwrap();
}
```

## 14. Implementation Checklist

### Primary Validator
- [ ] HTTP server with mTLS/API key auth
- [ ] Validation pipeline (all 9 stages)
- [ ] Schema fetching and caching
- [ ] Invariant checking (on-chain, oracle, app logic)
- [ ] Nonce tracking with Bridge sync
- [ ] EIP-712 signing
- [ ] Storage publication (Arweave, IPFS, GCS)
- [ ] Bridge interaction (initializeAndSign, rejectProposal)
- [ ] Health/status endpoints
- [ ] Rate limiting

### Witness Validator
- [ ] Event monitoring (MessageInitialized)
- [ ] Storage fetching with fallback
- [ ] Metadata re-derivation
- [ ] Verification API client
- [ ] Independent invariant checking
- [ ] Bridge interaction (signMessage, rejectMessage)

### Shared Components
- [ ] Configuration loading (TOML)
- [ ] TEE key management (GCP, AWS)
- [ ] Oracle client (Chainlink, Pyth)
- [ ] RPC client (multi-chain, retry)
- [ ] Error types and responses
- [ ] Logging (structured, JSON)
- [ ] Metrics (Prometheus)
- [ ] Graceful shutdown

### Testing
- [ ] Unit tests for validation pipeline
- [ ] Integration tests with mock Bridge
- [ ] End-to-end tests with testnet
- [ ] Load testing
- [ ] Chaos testing (storage failures, RPC failures)
