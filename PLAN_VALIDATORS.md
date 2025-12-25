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

## 2. Application Authentication

### 2.1 Authentication Methods

```rust
pub enum AuthMethod {
    ApiKey { key: String },
    MTls { client_cert: X509Certificate },
    TeeAttestation { token: String, platform: TeePlatform },
}

pub struct AuthConfig {
    pub method: AuthMethod,
    pub allowed_domains: Vec<[u8; 32]>,
    pub allowed_message_types: Vec<String>,
    pub rate_limit: RateLimitConfig,
}

pub struct RateLimitConfig {
    pub max_per_second: u32,
    pub max_per_day: u32,
}
```

### 2.2 mTLS Authentication

```rust
use rustls::{Certificate, RootCertStore};

pub struct MTlsAuth {
    client_ca: RootCertStore,
    registrations: HashMap<CertFingerprint, AuthConfig>,
}

impl MTlsAuth {
    pub fn verify(&self, cert: &Certificate) -> Result<AuthConfig, AuthError> {
        // Verify certificate chain against CA
        let chain = verify_certificate_chain(cert, &self.client_ca)?;

        // Extract fingerprint for lookup
        let fingerprint = compute_cert_fingerprint(cert);

        // Find registration for this client
        self.registrations.get(&fingerprint)
            .cloned()
            .ok_or(AuthError::UnknownClient(fingerprint))
    }
}

// Axum middleware for mTLS
pub async fn mtls_middleware<B>(
    State(auth): State<Arc<MTlsAuth>>,
    ConnectInfo(info): ConnectInfo<TlsConnectInfo>,
    request: Request<B>,
    next: Next<B>,
) -> Result<Response, StatusCode> {
    let cert = info.peer_certificate()
        .ok_or(StatusCode::UNAUTHORIZED)?;

    let config = auth.verify(&cert)
        .map_err(|_| StatusCode::FORBIDDEN)?;

    // Store config in request extensions
    let mut request = request;
    request.extensions_mut().insert(config);

    Ok(next.run(request).await)
}
```

### 2.3 TEE Attestation Headers

```rust
pub struct TeeAttestation {
    pub token: String,
    pub platform: TeePlatform,
    pub code_hash: [u8; 32],
}

#[derive(Clone, Copy)]
pub enum TeePlatform {
    GcpConfidentialSpace,
    AwsNitroEnclaves,
    AzureConfidentialVm,
    IntelSgx,
}

pub fn extract_tee_attestation(headers: &HeaderMap) -> Option<TeeAttestation> {
    let token = headers.get("X-TEE-Attestation")?.to_str().ok()?;
    let platform = headers.get("X-TEE-Platform")?.to_str().ok()?;

    let platform = match platform {
        "gcp-confidential-space" => TeePlatform::GcpConfidentialSpace,
        "aws-nitro" => TeePlatform::AwsNitroEnclaves,
        "azure-confidential" => TeePlatform::AzureConfidentialVm,
        "intel-sgx" => TeePlatform::IntelSgx,
        _ => return None,
    };

    Some(TeeAttestation {
        token: token.to_string(),
        platform,
        code_hash: [0u8; 32], // Extracted during verification
    })
}

pub async fn verify_tee_attestation(
    attestation: &TeeAttestation,
    expected_code_hash: &[u8; 32],
) -> Result<(), AuthError> {
    match attestation.platform {
        TeePlatform::GcpConfidentialSpace => {
            verify_gcp_attestation(&attestation.token, expected_code_hash).await
        }
        TeePlatform::AwsNitroEnclaves => {
            verify_aws_nitro_attestation(&attestation.token, expected_code_hash).await
        }
        // ... other platforms
    }
}
```

## 3. Error Types

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

/// RFC 8785 JSON Canonicalization Scheme (JCS)
///
/// Rules:
/// 1. Object keys sorted lexicographically by UTF-16 code units
/// 2. No whitespace between tokens
/// 3. Number formatting: no leading zeros, no trailing zeros after decimal, no positive sign
/// 4. String escaping: minimal escaping (only required characters)
/// 5. No duplicate keys
pub fn json_canonicalize(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Bool(b) => if *b { "true" } else { "false" }.to_string(),
        serde_json::Value::Number(n) => canonicalize_number(n),
        serde_json::Value::String(s) => canonicalize_string(s),
        serde_json::Value::Array(arr) => {
            let elements: Vec<String> = arr.iter()
                .map(|v| json_canonicalize(v))
                .collect();
            format!("[{}]", elements.join(","))
        }
        serde_json::Value::Object(obj) => {
            // Sort keys lexicographically (UTF-16 code units)
            let mut sorted_keys: Vec<&String> = obj.keys().collect();
            sorted_keys.sort_by(|a, b| {
                a.encode_utf16().cmp(b.encode_utf16())
            });

            let pairs: Vec<String> = sorted_keys.iter()
                .map(|k| {
                    let v = json_canonicalize(&obj[*k]);
                    format!("{}:{}", canonicalize_string(k), v)
                })
                .collect();
            format!("{{{}}}", pairs.join(","))
        }
    }
}

fn canonicalize_string(s: &str) -> String {
    let mut result = String::with_capacity(s.len() + 2);
    result.push('"');
    for c in s.chars() {
        match c {
            '"' => result.push_str("\\\""),
            '\\' => result.push_str("\\\\"),
            '\x08' => result.push_str("\\b"),
            '\x0c' => result.push_str("\\f"),
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            c if c < '\x20' => result.push_str(&format!("\\u{:04x}", c as u32)),
            c => result.push(c),
        }
    }
    result.push('"');
    result
}

fn canonicalize_number(n: &serde_json::Number) -> String {
    // RFC 8785: Use ES6 number serialization
    // - No leading zeros
    // - No trailing zeros after decimal
    // - No positive sign
    // - Use exponential notation for very large/small numbers
    if let Some(i) = n.as_i64() {
        i.to_string()
    } else if let Some(f) = n.as_f64() {
        // ES6 compatible formatting
        format_es6_number(f)
    } else {
        n.to_string()
    }
}

fn format_es6_number(n: f64) -> String {
    if n.is_nan() || n.is_infinite() {
        panic!("NaN and Infinity not allowed in canonical JSON");
    }
    // Use Rust's default float formatting which matches ES6 for most cases
    let s = format!("{}", n);
    // Ensure no trailing .0 for integers
    if s.ends_with(".0") && !s.contains('e') {
        s[..s.len()-2].to_string()
    } else {
        s
    }
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

### 5.2 storageRef URI Parsing

```rust
/// Parse pipe-separated storage URIs
/// Format: "ar://tx_id|ipfs://QmHash|gcs://bucket/path"
pub struct StorageRef {
    pub uris: Vec<StorageUri>,
}

#[derive(Clone)]
pub enum StorageUri {
    Arweave { tx_id: String },
    Ipfs { cid: String },
    Gcs { bucket: String, path: String },
}

impl StorageRef {
    pub fn parse(storage_ref: &str) -> Result<Self, Error> {
        let uris = storage_ref
            .split('|')
            .map(|uri| Self::parse_uri(uri.trim()))
            .collect::<Result<Vec<_>, _>>()?;

        if uris.is_empty() {
            return Err(Error::InvalidStorageRef("empty storage ref".into()));
        }

        Ok(Self { uris })
    }

    fn parse_uri(uri: &str) -> Result<StorageUri, Error> {
        if let Some(tx_id) = uri.strip_prefix("ar://") {
            Ok(StorageUri::Arweave { tx_id: tx_id.to_string() })
        } else if let Some(cid) = uri.strip_prefix("ipfs://") {
            Ok(StorageUri::Ipfs { cid: cid.to_string() })
        } else if let Some(path) = uri.strip_prefix("gcs://") {
            let parts: Vec<&str> = path.splitn(2, '/').collect();
            if parts.len() != 2 {
                return Err(Error::InvalidStorageRef(format!("invalid gcs path: {}", uri)));
            }
            Ok(StorageUri::Gcs {
                bucket: parts[0].to_string(),
                path: parts[1].to_string(),
            })
        } else {
            Err(Error::InvalidStorageRef(format!("unknown scheme: {}", uri)))
        }
    }
}
```

### 5.3 Storage Fetcher with Fallback

```rust
pub struct StorageFetcher {
    arweave: Option<ArweaveClient>,
    ipfs: Option<IpfsClient>,
    gcs: Option<GcsClient>,
    retry_config: RetryConfig,
}

impl StorageFetcher {
    /// Fetch message from storage with fallback through multiple URIs
    pub async fn fetch(&self, storage_ref: &str) -> Result<StorageRecord, Error> {
        let parsed = StorageRef::parse(storage_ref)?;

        // Try URIs in order of reliability
        let prioritized = self.prioritize_uris(&parsed.uris);

        let mut last_error = None;

        for uri in prioritized {
            match self.fetch_uri(&uri).await {
                Ok(record) => return Ok(record),
                Err(e) => {
                    tracing::warn!(uri = ?uri, error = %e, "Failed to fetch from storage");
                    last_error = Some(e);
                }
            }
        }

        Err(last_error.unwrap_or_else(|| Error::StorageFetchFailed("no valid URIs".into())))
    }

    fn prioritize_uris(&self, uris: &[StorageUri]) -> Vec<StorageUri> {
        // Priority: Arweave > IPFS > GCS
        let mut sorted = uris.to_vec();
        sorted.sort_by_key(|uri| match uri {
            StorageUri::Arweave { .. } => 0,
            StorageUri::Ipfs { .. } => 1,
            StorageUri::Gcs { .. } => 2,
        });
        sorted
    }

    async fn fetch_uri(&self, uri: &StorageUri) -> Result<StorageRecord, Error> {
        let json = match uri {
            StorageUri::Arweave { tx_id } => {
                let client = self.arweave.as_ref()
                    .ok_or_else(|| Error::StorageFetchFailed("arweave not configured".into()))?;
                self.with_retry(|| client.fetch(tx_id)).await?
            }
            StorageUri::Ipfs { cid } => {
                let client = self.ipfs.as_ref()
                    .ok_or_else(|| Error::StorageFetchFailed("ipfs not configured".into()))?;
                self.with_retry(|| client.fetch(cid)).await?
            }
            StorageUri::Gcs { bucket, path } => {
                let client = self.gcs.as_ref()
                    .ok_or_else(|| Error::StorageFetchFailed("gcs not configured".into()))?;
                self.with_retry(|| client.fetch(bucket, path)).await?
            }
        };

        serde_json::from_str(&json)
            .map_err(|e| Error::StorageFetchFailed(format!("invalid json: {}", e)))
    }

    async fn with_retry<F, Fut, T>(&self, f: F) -> Result<T, Error>
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = Result<T, Error>>,
    {
        let mut attempts = 0;
        loop {
            match f().await {
                Ok(result) => return Ok(result),
                Err(e) => {
                    attempts += 1;
                    if attempts >= self.retry_config.max_attempts {
                        return Err(e);
                    }
                    let delay = self.retry_config.base_delay * 2u32.pow(attempts - 1);
                    tokio::time::sleep(delay).await;
                }
            }
        }
    }
}

pub struct RetryConfig {
    pub max_attempts: u32,
    pub base_delay: Duration,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            base_delay: Duration::from_millis(500),
        }
    }
}
```

### 5.4 Metadata Re-Derivation

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

## 10. Schema Caching

### 10.1 Schema Cache

```rust
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct SchemaCache {
    cache: Arc<RwLock<HashMap<String, CachedSchema>>>,
    bridge: BridgeClient,
    storage: StorageFetcher,
    ttl: Duration,
}

struct CachedSchema {
    schema: jsonschema::JSONSchema,
    schema_hash: [u8; 32],
    fetched_at: Instant,
    uri: String,
}

impl SchemaCache {
    pub async fn get_or_fetch(&self, uri: &str) -> Result<Arc<jsonschema::JSONSchema>, Error> {
        // Check cache first
        {
            let cache = self.cache.read().await;
            if let Some(cached) = cache.get(uri) {
                if cached.fetched_at.elapsed() < self.ttl {
                    return Ok(Arc::new(cached.schema.clone()));
                }
            }
        }

        // Fetch from storage
        let json = self.storage.fetch_schema(uri).await?;
        let schema_hash = keccak256(json.as_bytes());

        let schema = jsonschema::JSONSchema::compile(&serde_json::from_str(&json)?)
            .map_err(|e| Error::InvalidSchema(e.to_string()))?;

        // Store in cache
        {
            let mut cache = self.cache.write().await;
            cache.insert(uri.to_string(), CachedSchema {
                schema: schema.clone(),
                schema_hash,
                fetched_at: Instant::now(),
                uri: uri.to_string(),
            });
        }

        Ok(Arc::new(schema))
    }

    pub async fn get_by_message_type(&self, message_type: &str) -> Result<Arc<jsonschema::JSONSchema>, Error> {
        let config = self.bridge.get_message_type_config(message_type).await?
            .ok_or_else(|| Error::MessageTypeNotRegistered(message_type.to_string()))?;

        self.get_or_fetch(&config.schema_uri).await
    }
}
```

### 10.2 Event-Based Cache Invalidation

```rust
pub struct SchemaEventMonitor {
    bridge: BridgeClient,
    cache: Arc<SchemaCache>,
}

impl SchemaEventMonitor {
    pub async fn run(&self, mut shutdown: watch::Receiver<bool>) -> Result<(), Error> {
        // Subscribe to schema update events
        let mut schema_events = self.bridge.subscribe_events("MessageTypeRegistered").await?;
        let mut update_events = self.bridge.subscribe_events("MessageTypeUpdated").await?;
        let mut enable_events = self.bridge.subscribe_events("MessageTypeEnabled").await?;

        loop {
            tokio::select! {
                Some(event) = schema_events.next() => {
                    self.handle_registered(event).await;
                }
                Some(event) = update_events.next() => {
                    self.handle_updated(event).await;
                }
                Some(event) = enable_events.next() => {
                    self.handle_enabled(event).await;
                }
                _ = shutdown.changed() => {
                    break;
                }
            }
        }
        Ok(())
    }

    async fn handle_registered(&self, event: MessageTypeRegisteredEvent) {
        tracing::info!(
            message_type = %event.message_type,
            schema_hash = %hex::encode(event.schema_hash),
            "New message type registered, pre-caching schema"
        );

        // Pre-fetch and cache the schema
        if let Err(e) = self.cache.get_by_message_type(&event.message_type).await {
            tracing::warn!(error = %e, "Failed to pre-cache schema");
        }
    }

    async fn handle_updated(&self, event: MessageTypeUpdatedEvent) {
        tracing::info!(
            message_type = %event.message_type,
            old_hash = %hex::encode(event.old_hash),
            new_hash = %hex::encode(event.new_hash),
            "Schema updated, invalidating cache"
        );

        // Invalidate cached schema
        self.cache.invalidate(&event.message_type).await;

        // Re-fetch new schema
        if let Err(e) = self.cache.get_by_message_type(&event.message_type).await {
            tracing::warn!(error = %e, "Failed to fetch updated schema");
        }
    }

    async fn handle_enabled(&self, event: MessageTypeEnabledEvent) {
        tracing::info!(
            message_type = %event.message_type,
            enabled = event.enabled,
            "Message type enabled state changed"
        );

        // Update local enabled state (doesn't affect schema cache)
        self.cache.set_enabled(&event.message_type, event.enabled).await;
    }
}

// Event types
struct MessageTypeRegisteredEvent {
    message_type: String,
    target: Address,
    schema_hash: [u8; 32],
}

struct MessageTypeUpdatedEvent {
    message_type: String,
    old_hash: [u8; 32],
    new_hash: [u8; 32],
}

struct MessageTypeEnabledEvent {
    message_type: String,
    enabled: bool,
}
```

### 10.3 Startup Schema Sync

```rust
impl SchemaCache {
    /// Sync all schemas from Bridge on startup
    pub async fn sync_from_bridge(&self) -> Result<(), Error> {
        tracing::info!("Syncing schemas from Bridge...");

        // Get all registered message types
        let message_types = self.bridge.get_all_message_types().await?;

        for message_type in &message_types {
            let config = self.bridge.get_message_type_config(message_type).await?;

            if let Some(config) = config {
                if config.enabled {
                    if let Err(e) = self.get_or_fetch(&config.schema_uri).await {
                        tracing::warn!(
                            message_type = %message_type,
                            error = %e,
                            "Failed to cache schema"
                        );
                    }
                }
            }
        }

        tracing::info!(count = message_types.len(), "Schema sync complete");
        Ok(())
    }

    pub async fn invalidate(&self, message_type: &str) {
        let mut cache = self.cache.write().await;
        // Find and remove by message type (need to look up URI first)
        cache.retain(|_, v| !v.uri.contains(message_type));
    }

    pub async fn set_enabled(&self, _message_type: &str, _enabled: bool) {
        // Track enabled state separately if needed for fast lookups
    }
}
```

## 11. DA Reference Tracking

```rust
/// Track DA publication references for audit and queries
pub struct DaReferenceTracker {
    db: Database,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DaPublication {
    pub message_id: String,
    pub da_layer: String,
    pub da_reference: String,
    pub published_at: u64,
    pub confirmed: bool,
}

impl DaReferenceTracker {
    /// Record a DA publication
    pub async fn record(&self, publication: &DaPublication) -> Result<(), Error> {
        self.db.execute(
            "INSERT INTO da_publications (message_id, da_layer, da_reference, published_at, confirmed)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(message_id, da_layer) DO UPDATE SET
                 da_reference = ?3, published_at = ?4, confirmed = ?5",
            &[
                &publication.message_id,
                &publication.da_layer,
                &publication.da_reference,
                &(publication.published_at as i64),
                &publication.confirmed,
            ],
        ).await
    }

    /// Mark a publication as confirmed
    pub async fn confirm(&self, message_id: &str, da_layer: &str) -> Result<(), Error> {
        self.db.execute(
            "UPDATE da_publications SET confirmed = true WHERE message_id = ?1 AND da_layer = ?2",
            &[message_id, da_layer],
        ).await
    }

    /// Get all publications for a message
    pub async fn get_publications(&self, message_id: &str) -> Result<Vec<DaPublication>, Error> {
        self.db.query(
            "SELECT message_id, da_layer, da_reference, published_at, confirmed
             FROM da_publications WHERE message_id = ?1",
            &[message_id],
        ).await
    }

    /// Get messages published in a time range
    pub async fn get_by_time_range(&self, start: u64, end: u64) -> Result<Vec<DaPublication>, Error> {
        self.db.query(
            "SELECT message_id, da_layer, da_reference, published_at, confirmed
             FROM da_publications WHERE published_at >= ?1 AND published_at <= ?2
             ORDER BY published_at DESC",
            &[&(start as i64), &(end as i64)],
        ).await
    }

    /// Database schema
    pub async fn create_tables(&self) -> Result<(), Error> {
        self.db.execute(
            "CREATE TABLE IF NOT EXISTS da_publications (
                message_id TEXT NOT NULL,
                da_layer TEXT NOT NULL,
                da_reference TEXT NOT NULL,
                published_at INTEGER NOT NULL,
                confirmed INTEGER DEFAULT 0,
                PRIMARY KEY (message_id, da_layer)
            )",
            &[],
        ).await?;

        self.db.execute(
            "CREATE INDEX IF NOT EXISTS idx_da_published_at ON da_publications(published_at)",
            &[],
        ).await?;

        Ok(())
    }
}
```

### 11.1 DA Query API Endpoint

```rust
// Add to HTTP handlers
async fn get_message_da(
    Path(message_id): Path<String>,
    State(state): State<AppState>,
) -> Result<Json<DaResponse>, StatusCode> {
    let publications = state.da_tracker.get_publications(&message_id).await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if publications.is_empty() {
        return Err(StatusCode::NOT_FOUND);
    }

    // Return primary (first confirmed, or first)
    let primary = publications.iter()
        .find(|p| p.confirmed)
        .or_else(|| publications.first())
        .unwrap();

    Ok(Json(DaResponse {
        layer: primary.da_layer.clone(),
        reference: primary.da_reference.clone(),
        published_at: primary.published_at,
        confirmed: primary.confirmed,
        all_layers: publications.iter().map(|p| LayerInfo {
            layer: p.da_layer.clone(),
            reference: p.da_reference.clone(),
            confirmed: p.confirmed,
        }).collect(),
    }))
}

#[derive(Serialize)]
struct DaResponse {
    layer: String,
    reference: String,
    published_at: u64,
    confirmed: bool,
    all_layers: Vec<LayerInfo>,
}

#[derive(Serialize)]
struct LayerInfo {
    layer: String,
    reference: String,
    confirmed: bool,
}
```

## 12. EIP-712 Signing

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

## 13. TEE Key Management

### 13.1 Key Generation in TEE

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

### 13.2 Attestation Verification

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

## 14. Configuration

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

## 15. Metrics & Observability

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

## 16. Graceful Shutdown

### 16.1 Shutdown Coordinator

```rust
use tokio::sync::{broadcast, watch};

pub struct ShutdownCoordinator {
    /// Signal to begin shutdown
    shutdown_tx: broadcast::Sender<()>,
    /// Current state
    state: Arc<RwLock<ShutdownState>>,
    /// Drain timeout
    drain_timeout: Duration,
}

#[derive(Clone, Copy, PartialEq)]
pub enum ShutdownState {
    Running,
    Draining,      // Stop accepting new requests, complete in-flight
    ShuttingDown,  // Force terminate remaining
    Terminated,
}

impl ShutdownCoordinator {
    pub fn new(drain_timeout: Duration) -> Self {
        let (shutdown_tx, _) = broadcast::channel(1);
        Self {
            shutdown_tx,
            state: Arc::new(RwLock::new(ShutdownState::Running)),
            drain_timeout,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<()> {
        self.shutdown_tx.subscribe()
    }

    pub async fn initiate_shutdown(&self) {
        tracing::info!("Initiating graceful shutdown");

        // Transition to draining
        {
            let mut state = self.state.write().await;
            *state = ShutdownState::Draining;
        }

        // Notify all subscribers
        let _ = self.shutdown_tx.send(());

        // Wait for drain timeout
        tracing::info!(timeout = ?self.drain_timeout, "Waiting for in-flight requests to complete");
        tokio::time::sleep(self.drain_timeout).await;

        // Force shutdown
        {
            let mut state = self.state.write().await;
            *state = ShutdownState::ShuttingDown;
        }

        tracing::info!("Drain timeout reached, forcing shutdown");
    }

    pub async fn is_accepting_requests(&self) -> bool {
        *self.state.read().await == ShutdownState::Running
    }
}
```

### 16.2 In-Flight Request Tracking

```rust
pub struct InFlightTracker {
    /// Number of requests currently being processed
    count: AtomicU64,
    /// Notify when count reaches zero
    zero_notify: Notify,
}

impl InFlightTracker {
    pub fn new() -> Self {
        Self {
            count: AtomicU64::new(0),
            zero_notify: Notify::new(),
        }
    }

    pub fn start_request(&self) -> InFlightGuard {
        self.count.fetch_add(1, Ordering::SeqCst);
        PENDING_MESSAGES.inc();
        InFlightGuard { tracker: self }
    }

    pub async fn wait_for_zero(&self, timeout: Duration) -> bool {
        if self.count.load(Ordering::SeqCst) == 0 {
            return true;
        }

        tokio::select! {
            _ = self.zero_notify.notified() => true,
            _ = tokio::time::sleep(timeout) => false,
        }
    }

    fn decrement(&self) {
        let prev = self.count.fetch_sub(1, Ordering::SeqCst);
        PENDING_MESSAGES.dec();
        if prev == 1 {
            self.zero_notify.notify_waiters();
        }
    }
}

pub struct InFlightGuard<'a> {
    tracker: &'a InFlightTracker,
}

impl Drop for InFlightGuard<'_> {
    fn drop(&mut self) {
        self.tracker.decrement();
    }
}
```

### 16.3 HTTP Server Graceful Shutdown

```rust
pub async fn run_server(
    config: &Config,
    shutdown: ShutdownCoordinator,
    in_flight: Arc<InFlightTracker>,
) -> Result<(), Error> {
    let app = create_router(config)
        .layer(Extension(shutdown.clone()))
        .layer(Extension(in_flight.clone()));

    let addr = config.http.bind.parse()?;
    let listener = TcpListener::bind(addr).await?;

    tracing::info!(%addr, "HTTP server starting");

    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let mut rx = shutdown.subscribe();
            let _ = rx.recv().await;
            tracing::info!("HTTP server received shutdown signal");

            // Wait for in-flight requests
            let drained = in_flight.wait_for_zero(shutdown.drain_timeout).await;
            if drained {
                tracing::info!("All in-flight requests completed");
            } else {
                tracing::warn!(
                    remaining = in_flight.count.load(Ordering::SeqCst),
                    "Drain timeout, some requests may be interrupted"
                );
            }
        })
        .await?;

    Ok(())
}

// Middleware to reject new requests during shutdown
pub async fn shutdown_guard<B>(
    Extension(shutdown): Extension<ShutdownCoordinator>,
    Extension(in_flight): Extension<Arc<InFlightTracker>>,
    request: Request<B>,
    next: Next<B>,
) -> Result<Response, StatusCode> {
    if !shutdown.is_accepting_requests().await {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }

    let _guard = in_flight.start_request();
    Ok(next.run(request).await)
}
```

### 16.4 Background Task Shutdown

```rust
pub struct TaskManager {
    tasks: Vec<JoinHandle<()>>,
    shutdown: ShutdownCoordinator,
}

impl TaskManager {
    pub fn spawn<F>(&mut self, name: &str, future: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let name = name.to_string();
        let handle = tokio::spawn(async move {
            future.await;
            tracing::debug!(task = %name, "Background task completed");
        });
        self.tasks.push(handle);
    }

    pub async fn shutdown_all(&mut self, timeout: Duration) {
        tracing::info!(count = self.tasks.len(), "Shutting down background tasks");

        // Wait for all tasks with timeout
        let shutdown_future = async {
            for handle in &mut self.tasks {
                let _ = handle.await;
            }
        };

        match tokio::time::timeout(timeout, shutdown_future).await {
            Ok(_) => tracing::info!("All background tasks completed"),
            Err(_) => {
                tracing::warn!("Task shutdown timeout, aborting remaining tasks");
                for handle in &self.tasks {
                    handle.abort();
                }
            }
        }
    }
}
```

### 16.5 Main Shutdown Flow

```rust
#[tokio::main]
async fn main() -> Result<(), Error> {
    // Setup
    let config = Config::load()?;
    let shutdown = ShutdownCoordinator::new(Duration::from_secs(30));
    let in_flight = Arc::new(InFlightTracker::new());

    // Spawn background tasks
    let mut tasks = TaskManager::new(shutdown.clone());
    tasks.spawn("event_monitor", run_event_monitor(config.clone(), shutdown.subscribe()));
    tasks.spawn("schema_sync", run_schema_sync(config.clone(), shutdown.subscribe()));
    tasks.spawn("nonce_sync", run_nonce_sync(config.clone(), shutdown.subscribe()));

    // Setup signal handlers
    let shutdown_clone = shutdown.clone();
    tokio::spawn(async move {
        let mut sigterm = signal(SignalKind::terminate()).unwrap();
        let mut sigint = signal(SignalKind::interrupt()).unwrap();

        tokio::select! {
            _ = sigterm.recv() => tracing::info!("Received SIGTERM"),
            _ = sigint.recv() => tracing::info!("Received SIGINT"),
        }

        shutdown_clone.initiate_shutdown().await;
    });

    // Run HTTP server
    let server_result = run_server(&config, shutdown.clone(), in_flight).await;

    // Shutdown background tasks
    tasks.shutdown_all(Duration::from_secs(10)).await;

    // Flush metrics/logs
    tracing::info!("Validator shutdown complete");

    server_result
}
```

## 17. Key Rotation

### 17.1 Key Rotation Strategy

```rust
/// Key rotation without downtime requires:
/// 1. Generate new key in TEE
/// 2. Register new key on Bridge (addWitnessValidator or setPrimaryValidator)
/// 3. Wait for Bridge confirmation
/// 4. Switch to new key
/// 5. Remove old key from Bridge
pub struct KeyRotationManager {
    current_key: Arc<RwLock<SigningKey>>,
    pending_key: Arc<RwLock<Option<PendingKey>>>,
    bridge: BridgeClient,
    tee: TeeKeyManager,
}

struct PendingKey {
    key: SigningKey,
    registered_at: Option<u64>,  // Block number when registered
    confirmation_blocks: u64,
}

struct SigningKey {
    key_id: String,
    address: Address,
    created_at: u64,
}
```

### 17.2 Rotation Flow

```rust
impl KeyRotationManager {
    /// Initiate key rotation (Step 1-2)
    pub async fn initiate_rotation(&self) -> Result<Address, Error> {
        // Check no pending rotation
        if self.pending_key.read().await.is_some() {
            return Err(Error::RotationAlreadyPending);
        }

        tracing::info!("Initiating key rotation");

        // Generate new key in TEE
        let tee_key = self.tee.generate_signing_key().await?;

        tracing::info!(
            new_address = %tee_key.address,
            "Generated new signing key in TEE"
        );

        // Register on Bridge
        let tx_hash = self.bridge.add_witness_validator(
            tee_key.address,
            &tee_key.attestation,
        ).await?;

        tracing::info!(
            tx = %tx_hash,
            "Submitted new validator registration to Bridge"
        );

        // Wait for confirmation
        let receipt = self.bridge.wait_for_receipt(tx_hash).await?;

        // Store pending key
        {
            let mut pending = self.pending_key.write().await;
            *pending = Some(PendingKey {
                key: SigningKey {
                    key_id: tee_key.key_id,
                    address: tee_key.address,
                    created_at: receipt.block_number,
                },
                registered_at: Some(receipt.block_number),
                confirmation_blocks: 12,  // Wait 12 blocks
            });
        }

        Ok(tee_key.address)
    }

    /// Complete rotation after confirmation (Step 3-4)
    pub async fn complete_rotation(&self) -> Result<(), Error> {
        let pending = self.pending_key.read().await.clone()
            .ok_or(Error::NoPendingRotation)?;

        // Check confirmation
        let current_block = self.bridge.get_block_number().await?;
        let registered_at = pending.registered_at.ok_or(Error::NotRegistered)?;

        if current_block < registered_at + pending.confirmation_blocks {
            return Err(Error::InsufficientConfirmations {
                current: current_block,
                required: registered_at + pending.confirmation_blocks,
            });
        }

        tracing::info!(
            old_address = %self.current_key.read().await.address,
            new_address = %pending.key.address,
            "Switching to new signing key"
        );

        // Swap keys
        let old_key = {
            let mut current = self.current_key.write().await;
            let old = current.clone();
            *current = pending.key.clone();
            old
        };

        // Clear pending
        {
            let mut pending_lock = self.pending_key.write().await;
            *pending_lock = None;
        }

        // Remove old key from Bridge (non-blocking)
        let bridge = self.bridge.clone();
        let old_address = old_key.address;
        tokio::spawn(async move {
            if let Err(e) = bridge.remove_validator(old_address).await {
                tracing::error!(error = %e, "Failed to remove old validator from Bridge");
            } else {
                tracing::info!(address = %old_address, "Removed old validator from Bridge");
            }
        });

        Ok(())
    }

    /// Cancel pending rotation
    pub async fn cancel_rotation(&self) -> Result<(), Error> {
        let pending = self.pending_key.write().await.take()
            .ok_or(Error::NoPendingRotation)?;

        tracing::info!(address = %pending.key.address, "Cancelling key rotation");

        // Remove pending key from Bridge if registered
        if pending.registered_at.is_some() {
            self.bridge.remove_validator(pending.key.address).await?;
        }

        Ok(())
    }

    /// Get signing key (for EIP-712 signing)
    pub async fn get_current_key(&self) -> SigningKey {
        self.current_key.read().await.clone()
    }
}
```

### 17.3 Automated Rotation Schedule

```rust
pub struct AutoRotationConfig {
    /// Rotate keys every N days
    pub rotation_interval_days: u32,
    /// Alert N days before rotation
    pub alert_before_days: u32,
    /// Perform rotation automatically (vs manual approval)
    pub auto_approve: bool,
}

pub async fn run_auto_rotation(
    config: AutoRotationConfig,
    rotation_manager: Arc<KeyRotationManager>,
    mut shutdown: broadcast::Receiver<()>,
) {
    let rotation_interval = Duration::from_secs(config.rotation_interval_days as u64 * 86400);
    let check_interval = Duration::from_secs(3600);  // Check hourly

    loop {
        tokio::select! {
            _ = tokio::time::sleep(check_interval) => {
                let current_key = rotation_manager.get_current_key().await;
                let age = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs() - current_key.created_at;

                let age_duration = Duration::from_secs(age);
                let alert_threshold = rotation_interval - Duration::from_secs(config.alert_before_days as u64 * 86400);

                if age_duration > rotation_interval {
                    tracing::warn!(
                        key_age_days = age / 86400,
                        "Key rotation overdue!"
                    );

                    if config.auto_approve {
                        match rotation_manager.initiate_rotation().await {
                            Ok(addr) => {
                                tracing::info!(new_address = %addr, "Auto-rotation initiated");
                                // Wait and complete
                                tokio::time::sleep(Duration::from_secs(180)).await;
                                if let Err(e) = rotation_manager.complete_rotation().await {
                                    tracing::error!(error = %e, "Auto-rotation completion failed");
                                }
                            }
                            Err(e) => {
                                tracing::error!(error = %e, "Auto-rotation failed");
                            }
                        }
                    }
                } else if age_duration > alert_threshold {
                    tracing::info!(
                        key_age_days = age / 86400,
                        rotation_due_days = (rotation_interval.as_secs() - age) / 86400,
                        "Key rotation approaching"
                    );
                }
            }
            _ = shutdown.recv() => {
                tracing::info!("Auto-rotation task shutting down");
                break;
            }
        }
    }
}
```

### 17.4 Rotation HTTP Endpoints

```rust
// POST /admin/keys/rotate/initiate
async fn initiate_key_rotation(
    State(state): State<AppState>,
) -> Result<Json<RotationResponse>, StatusCode> {
    let new_address = state.rotation_manager.initiate_rotation().await
        .map_err(|e| {
            tracing::error!(error = %e, "Key rotation initiation failed");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(RotationResponse {
        status: "pending",
        new_address: new_address.to_string(),
        message: "Rotation initiated. Call /complete after 12 block confirmations.",
    }))
}

// POST /admin/keys/rotate/complete
async fn complete_key_rotation(
    State(state): State<AppState>,
) -> Result<Json<RotationResponse>, StatusCode> {
    state.rotation_manager.complete_rotation().await
        .map_err(|e| {
            tracing::error!(error = %e, "Key rotation completion failed");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let current = state.rotation_manager.get_current_key().await;

    Ok(Json(RotationResponse {
        status: "completed",
        new_address: current.address.to_string(),
        message: "Key rotation completed. Old key removed from Bridge.",
    }))
}

// DELETE /admin/keys/rotate
async fn cancel_key_rotation(
    State(state): State<AppState>,
) -> Result<Json<RotationResponse>, StatusCode> {
    state.rotation_manager.cancel_rotation().await
        .map_err(|e| {
            tracing::error!(error = %e, "Key rotation cancellation failed");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(RotationResponse {
        status: "cancelled",
        new_address: String::new(),
        message: "Pending key rotation cancelled.",
    }))
}

// GET /admin/keys/status
async fn get_key_status(
    State(state): State<AppState>,
) -> Json<KeyStatusResponse> {
    let current = state.rotation_manager.get_current_key().await;
    let pending = state.rotation_manager.pending_key.read().await.clone();

    let age = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() - current.created_at;

    Json(KeyStatusResponse {
        current_address: current.address.to_string(),
        key_age_seconds: age,
        pending_rotation: pending.map(|p| PendingRotationInfo {
            new_address: p.key.address.to_string(),
            registered_at_block: p.registered_at,
            confirmations_required: p.confirmation_blocks,
        }),
    })
}
```

## 18. Implementation Checklist

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

### Authentication (Section 2)
- [ ] mTLS authentication middleware
- [ ] API key authentication
- [ ] TEE attestation header extraction
- [ ] Client registration storage

### JSON Canonicalization (Section 4.2)
- [ ] RFC 8785 implementation
- [ ] UTF-16 key sorting
- [ ] Number normalization
- [ ] String escaping

### storageRef Parsing (Section 5.2-5.3)
- [ ] URI parsing (ar://, ipfs://, gcs://)
- [ ] Pipe-separated multi-URI support
- [ ] Storage fetcher with fallback
- [ ] Retry with exponential backoff

### Schema Caching (Section 10)
- [ ] Schema cache with TTL
- [ ] Event-based invalidation
- [ ] Startup schema sync
- [ ] jsonschema validation

### DA Reference Tracking (Section 11)
- [ ] SQLite table for publications
- [ ] Record/confirm operations
- [ ] Time-range queries
- [ ] API endpoint (/messages/{id}/da)

### Graceful Shutdown (Section 16)
- [ ] ShutdownCoordinator with state machine
- [ ] InFlightTracker with guard pattern
- [ ] HTTP server graceful shutdown
- [ ] Background task shutdown with timeout
- [ ] Signal handlers (SIGTERM, SIGINT)
- [ ] Drain timeout configuration

### Key Rotation (Section 17)
- [ ] KeyRotationManager with pending state
- [ ] initiate_rotation() with TEE and Bridge
- [ ] complete_rotation() with confirmation check
- [ ] cancel_rotation()
- [ ] Auto-rotation scheduler
- [ ] HTTP endpoints (/admin/keys/*)
- [ ] Key age metrics

### Testing
- [ ] Unit tests for validation pipeline
- [ ] Integration tests with mock Bridge
- [ ] End-to-end tests with testnet
- [ ] Load testing
- [ ] Chaos testing (storage failures, RPC failures)
