# PLAN_REPLICA.md - Read Replica and Validator Implementation

## Overview

The synddb-replica serves dual purposes: as a permissionless read replica that syncs from DA layers and serves queries, and as a validator when run in TEE mode with settlement capabilities. The same binary operates in different modes based on configuration, providing a unified codebase for both read serving and validation.

## Architecture

```
┌────────────────────────────────────────────────────────────────┐
│                         DA Layers                              │
│  ┌──────────┐ ┌──────────┐ ┌──────┐ ┌──────────┐            │
│  │ Celestia │ │ EigenDA  │ │ IPFS │ │ Arweave  │            │
│  └──────────┘ └──────────┘ └──────┘ └──────────┘            │
└────────────────────────────────────────────────────────────────┘
                    ↓                ↓
┌────────────────────────────────────────────────────────────────┐
│                    synddb-replica                              │
│ ┌──────────────────────────────────────────────────────────┐  │
│ │                    DA Syncer                              │  │
│ │  ┌────────────┐  ┌────────────┐  ┌────────────┐        │  │
│ │  │ Fetcher    │→ │ Verifier   │→ │  Orderer   │        │  │
│ │  └────────────┘  └────────────┘  └────────────┘        │  │
│ └──────────────────────────────────────────────────────────┘  │
│                           ↓                                    │
│ ┌──────────────────────────────────────────────────────────┐  │
│ │                   SQL Replayer                            │  │
│ │  ┌────────────┐  ┌────────────┐  ┌────────────┐        │  │
│ │  │ Parser     │→ │ Executor   │→ │ Validator  │        │  │
│ │  └────────────┘  └────────────┘  └────────────┘        │  │
│ └──────────────────────────────────────────────────────────┘  │
│                           ↓                                    │
│        ┌──────────────────────────────────┐                   │
│        │        Local SQLite DB           │                   │
│        └──────────────────────────────────┘                   │
│                    ↓              ↓                            │
│ ┌─────────────────────┐  ┌──────────────────────────────┐    │
│ │   Query Server      │  │   Validator Mode (TEE)       │    │
│ │  ┌──────────────┐  │  │  ┌────────────────────────┐  │    │
│ │  │  JSON-RPC    │  │  │  │ Message Processor       │  │    │
│ │  └──────────────┘  │  │  └────────────────────────┘  │    │
│ │  ┌──────────────┐  │  │  ┌────────────────────────┐  │    │
│ │  │  REST API    │  │  │  │ Settlement Poster       │  │    │
│ │  └──────────────┘  │  │  └────────────────────────┘  │    │
│ │  ┌──────────────┐  │  │  ┌────────────────────────┐  │    │
│ │  │  WebSocket   │  │  │  │ TEE Attestation        │  │    │
│ │  └──────────────┘  │  │  └────────────────────────┘  │    │
│ └─────────────────────┘  └──────────────────────────────┘    │
└────────────────────────────────────────────────────────────────┘
```

## Core Libraries

```toml
[dependencies]
# SQLite
rusqlite = { version = "0.32", features = ["bundled", "backup", "vtab", "hooks"] }
sqlite-parser = "0.5"  # Parse and validate SQL

# DA Layer clients
celestia-client = "0.2"
eigenda-rust = "0.1"
ipfs-api = "0.11"
arweave-rs = "0.1"

# Blockchain interaction
alloy = { version = "0.1", features = ["full"] }  # Ethereum interaction
ethers-signers = "2.0"  # Transaction signing

# Async runtime
tokio = { version = "1.35", features = ["full"] }
futures = "0.3"
async-trait = "0.1"

# API servers
axum = { version = "0.7", features = ["ws"] }  # REST and WebSocket
jsonrpsee = { version = "0.22", features = ["server", "macros"] }  # JSON-RPC
tower = "0.4"
tower-http = { version = "0.5", features = ["cors", "trace"] }

# Data handling
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
bincode = "1.3"
zstd = "0.13"

# TEE support
sgx-isa = { version = "0.4", optional = true }
dcap-ql = { version = "0.3", optional = true }
teaclave-attestation = { version = "0.5", optional = true }

# State management
blake3 = "1.5"  # Fast hashing for state updates

# Monitoring and logging
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
prometheus = "0.13"
opentelemetry = { version = "0.22", features = ["rt-tokio"] }

# Configuration
config = "0.14"
clap = { version = "4.4", features = ["derive", "env"] }

# Error handling
anyhow = "1.0"
thiserror = "1.0"

# Utilities
parking_lot = "0.12"
dashmap = "5.5"  # Concurrent hashmap
crossbeam = "0.8"
backoff = "0.4"
once_cell = "1.19"
```

## Directory Structure

```
synddb-replica/
├── Cargo.toml
├── src/
│   ├── main.rs                      # Entry point
│   ├── lib.rs                       # Public API
│   ├── config.rs                    # Configuration
│   ├── sync/
│   │   ├── mod.rs                   # DA syncing orchestration
│   │   ├── fetcher.rs               # Fetch from DA layers
│   │   ├── verifier.rs              # Verify data integrity
│   │   ├── orderer.rs               # Order operations for replay
│   │   ├── state_manager.rs         # Track sync state
│   │   └── providers/
│   │       ├── celestia.rs          # Celestia fetcher
│   │       ├── eigenda.rs           # EigenDA fetcher
│   │       ├── ipfs.rs              # IPFS fetcher
│   │       └── arweave.rs           # Arweave fetcher
│   ├── replay/
│   │   ├── mod.rs                   # SQL replay engine
│   │   ├── parser.rs                # Parse SQL operations
│   │   ├── executor.rs              # Execute on SQLite
│   │   ├── validator.rs             # Validate operations
│   │   ├── invariants.rs            # Check invariants
│   │   └── hooks.rs                 # Custom validation hooks
│   ├── database/
│   │   ├── mod.rs                   # SQLite management
│   │   ├── connection_pool.rs       # Connection pooling
│   │   ├── migrations.rs            # Schema management
│   │   ├── snapshots.rs             # Snapshot handling
│   │   └── vtables.rs               # Virtual tables
│   ├── api/
│   │   ├── mod.rs                   # API servers
│   │   ├── jsonrpc/
│   │   │   ├── mod.rs               # JSON-RPC server
│   │   │   ├── methods.rs           # RPC methods
│   │   │   └── types.rs             # RPC types
│   │   ├── rest/
│   │   │   ├── mod.rs               # REST server
│   │   │   ├── routes.rs            # HTTP routes
│   │   │   └── handlers.rs          # Request handlers
│   │   └── websocket/
│   │       ├── mod.rs               # WebSocket server
│   │       └── subscriptions.rs     # Real-time updates
│   ├── validator/
│   │   ├── mod.rs                   # Validator mode
│   │   ├── message_processor.rs     # Process messages
│   │   ├── settlement.rs            # Post to blockchain
│   │   ├── signature.rs             # Sign attestations
│   │   ├── consensus.rs             # Multi-validator coordination
│   │   └── extensions/
│   │       ├── mod.rs               # Extension points
│   │       ├── custom_rules.rs      # Custom validation
│   │       └── oracle_verify.rs     # External data verification
│   ├── tee/
│   │   ├── mod.rs                   # TEE integration
│   │   ├── enclave.rs               # Enclave management
│   │   ├── attestation.rs           # Generate attestations
│   │   ├── key_manager.rs           # Key management
│   │   └── remote_attestation.rs    # Remote attestation
│   ├── metrics/
│   │   ├── mod.rs                   # Metrics collection
│   │   └── collectors.rs            # Custom collectors
│   └── utils/
│       ├── mod.rs
│       ├── hash.rs                  # State hashing utilities
│       └── codec.rs                 # Encoding/decoding
├── config/
│   ├── replica.yaml                 # Read replica config
│   ├── validator.yaml               # Validator config
│   └── example.yaml                 # Full example
├── tests/
│   ├── integration/
│   └── benchmarks/
└── README.md
```

## Core Components

### 1. DA Syncer

Fetches data from multiple DA layers and maintains sync state:

```rust
// src/sync/mod.rs
pub struct DaSyncer {
    providers: Vec<Box<dyn DaProvider>>,
    state_manager: StateManager,
    verifier: DataVerifier,
    orderer: OperationOrderer,
}

#[async_trait]
pub trait DaProvider: Send + Sync {
    async fn fetch_range(&self, start: u64, end: u64) -> Result<Vec<DataPacket>>;
    async fn fetch_latest(&self) -> Result<u64>;
    fn name(&self) -> &str;
}

impl DaSyncer {
    pub async fn start(mut self, tx: Sender<SqlBatch>) -> Result<()> {
        loop {
            // Fetch from all providers
            let latest_sequence = self.get_latest_sequence().await?;
            let local_sequence = self.state_manager.get_sequence()?;
            
            if latest_sequence > local_sequence {
                let packets = self.fetch_missing(local_sequence + 1, latest_sequence).await?;
                
                // Verify and order operations
                let verified = self.verifier.verify_all(packets)?;
                let ordered = self.orderer.order(verified)?;

                // Send to replay engine
                for batch in ordered {
                    tx.send(batch).await?;
                }
                
                self.state_manager.update_sequence(latest_sequence)?;
            }
            
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }
    
    async fn fetch_missing(&self, start: u64, end: u64) -> Result<Vec<DataPacket>> {
        // Try primary provider first
        if let Some(primary) = self.providers.first() {
            if let Ok(packets) = primary.fetch_range(start, end).await {
                return Ok(packets);
            }
        }
        
        // Fallback to other providers
        for provider in &self.providers[1..] {
            if let Ok(packets) = provider.fetch_range(start, end).await {
                return Ok(packets);
            }
        }
        
        Err(anyhow!("Failed to fetch from all providers"))
    }
}

// src/sync/providers/celestia.rs
pub struct CelestiaProvider {
    client: CelestiaClient,
    namespace: Namespace,
}

#[async_trait]
impl DaProvider for CelestiaProvider {
    async fn fetch_range(&self, start: u64, end: u64) -> Result<Vec<DataPacket>> {
        let mut packets = Vec::new();
        
        for height in start..=end {
            let blobs = self.client
                .get_blobs_by_namespace(height, self.namespace)
                .await?;
                
            for blob in blobs {
                let packet = DataPacket::from_celestia_blob(blob)?;
                packets.push(packet);
            }
        }
        
        Ok(packets)
    }
}
```

### 2. SQL Replayer

Parses and executes SQL operations with validation:

```rust
// src/replay/mod.rs
pub struct SqlReplayer {
    executor: SqlExecutor,
    validator: OperationValidator,
    invariant_checker: InvariantChecker,
    hooks: Vec<Box<dyn ValidationHook>>,
}

impl SqlReplayer {
    pub async fn run(mut self, rx: Receiver<SqlBatch>) -> Result<()> {
        while let Some(batch) = rx.recv().await {
            self.replay_batch(batch).await?;
        }
        Ok(())
    }
    
    async fn replay_batch(&mut self, batch: SqlBatch) -> Result<()> {
        // Start transaction
        let mut tx = self.executor.begin_transaction()?;
        
        for operation in batch.operations {
            // Parse SQL
            let parsed = sqlite_parser::parse(&operation.sql)?;
            
            // Validate operation
            self.validator.validate(&parsed)?;
            
            // Run pre-execution hooks
            for hook in &self.hooks {
                hook.pre_execute(&parsed, &tx)?;
            }
            
            // Execute
            let result = tx.execute(&operation.sql)?;
            
            // Run post-execution hooks
            for hook in &self.hooks {
                hook.post_execute(&parsed, &result, &tx)?;
            }
        }
        
        // Check invariants before commit
        self.invariant_checker.check(&tx)?;
        
        // Commit transaction
        tx.commit()?;
        
        // Update state hash
        self.update_state_hash().await?;
        
        Ok(())
    }
}

// src/replay/validator.rs
pub struct OperationValidator {
    allowed_tables: HashSet<String>,
    max_row_changes: usize,
    custom_rules: Vec<Box<dyn ValidationRule>>,
}

impl OperationValidator {
    pub fn validate(&self, parsed: &ParsedSql) -> Result<()> {
        // Check table access
        for table in parsed.tables() {
            if !self.allowed_tables.contains(table) {
                return Err(anyhow!("Access to table {} not allowed", table));
            }
        }
        
        // Check operation type
        match parsed.statement_type() {
            StatementType::Select => {
                return Err(anyhow!("SELECT operations should not be replayed"));
            }
            StatementType::Drop | StatementType::Truncate => {
                return Err(anyhow!("Destructive operations not allowed"));
            }
            _ => {}
        }
        
        // Apply custom rules
        for rule in &self.custom_rules {
            rule.validate(parsed)?;
        }
        
        Ok(())
    }
}

// src/replay/invariants.rs
pub struct InvariantChecker {
    checks: Vec<Box<dyn InvariantCheck>>,
}

pub struct BalanceInvariant;

impl InvariantCheck for BalanceInvariant {
    fn check(&self, tx: &Transaction) -> Result<()> {
        // Check no negative balances
        let negative_count: i32 = tx.query_row(
            "SELECT COUNT(*) FROM balances WHERE amount < 0",
            [],
            |row| row.get(0)
        )?;
        
        if negative_count > 0 {
            return Err(anyhow!("Negative balance detected"));
        }
        
        // Check total supply matches
        let total: i64 = tx.query_row(
            "SELECT SUM(amount) FROM balances",
            [],
            |row| row.get(0)
        )?;
        
        let expected: i64 = tx.query_row(
            "SELECT total_supply FROM tokens",
            [],
            |row| row.get(0)
        )?;
        
        if total != expected {
            return Err(anyhow!("Balance sum mismatch"));
        }
        
        Ok(())
    }
}
```

### 3. Query Server

Serves queries via multiple protocols:

```rust
// src/api/jsonrpc/mod.rs
pub struct JsonRpcServer {
    db_pool: SqlitePool,
    methods: RpcMethods,
}

#[rpc(server)]
pub trait SyndDbRpc {
    #[method(name = "query")]
    async fn query(&self, sql: String) -> Result<QueryResult, Error>;
    
    #[method(name = "getStateHash")]
    async fn get_state_hash(&self) -> Result<String, Error>;
    
    #[method(name = "getSequence")]
    async fn get_sequence(&self) -> Result<u64, Error>;
    
    #[method(name = "subscribe")]
    async fn subscribe(&self, table: String) -> Result<SubscriptionId, Error>;
}

impl SyndDbRpcServer for JsonRpcServer {
    async fn query(&self, sql: String) -> Result<QueryResult, Error> {
        // Only allow SELECT queries
        if !sql.trim().to_uppercase().starts_with("SELECT") {
            return Err(Error::Custom("Only SELECT queries allowed".into()));
        }
        
        let conn = self.db_pool.get().await?;
        let rows = conn.query(&sql)?;
        
        Ok(QueryResult {
            columns: rows.columns(),
            rows: rows.to_json(),
        })
    }
}

// src/api/rest/mod.rs
pub struct RestServer {
    db_pool: SqlitePool,
}

pub fn routes(state: Arc<RestServer>) -> Router {
    Router::new()
        .route("/query", post(query_handler))
        .route("/tables", get(list_tables))
        .route("/table/:name", get(describe_table))
        .route("/health", get(health_check))
        .route("/metrics", get(prometheus_metrics))
        .with_state(state)
}

async fn query_handler(
    State(server): State<Arc<RestServer>>,
    Json(query): Json<QueryRequest>,
) -> Result<Json<QueryResponse>> {
    let result = server.execute_query(query.sql).await?;
    Ok(Json(result))
}

// src/api/websocket/mod.rs
pub struct WebSocketServer {
    db_pool: SqlitePool,
    subscriptions: Arc<DashMap<String, Vec<WebSocketSink>>>,
}

impl WebSocketServer {
    pub async fn handle_connection(&self, ws: WebSocket) {
        let (mut sender, mut receiver) = ws.split();
        
        while let Some(Ok(msg)) = receiver.next().await {
            match msg {
                Message::Text(text) => {
                    let request: WsRequest = serde_json::from_str(&text)?;
                    self.handle_request(request, &mut sender).await?;
                }
                Message::Close(_) => break,
                _ => {}
            }
        }
    }
    
    async fn handle_request(&self, req: WsRequest, sender: &mut WebSocketSink) {
        match req {
            WsRequest::Subscribe { table } => {
                self.subscriptions.entry(table).or_default().push(sender.clone());
            }
            WsRequest::Query { sql } => {
                let result = self.execute_query(sql).await?;
                sender.send(Message::Text(serde_json::to_string(&result)?)).await?;
            }
        }
    }
}
```

### 4. Validator Mode

Additional functionality when running as a validator:

```rust
// src/validator/mod.rs
pub struct ValidatorMode {
    message_processor: MessageProcessor,
    settlement_poster: SettlementPoster,
    attestor: TeeAttestor,
    consensus: ConsensusManager,
}

impl ValidatorMode {
    pub async fn start(self) -> Result<()> {
        tokio::select! {
            _ = self.process_messages() => {},
            _ = self.post_settlements() => {},
            _ = self.participate_consensus() => {},
        }
        Ok(())
    }
    
    async fn process_messages(&self) -> Result<()> {
        loop {
            // Monitor message tables
            let messages = self.message_processor.check_tables().await?;
            
            for message in messages {
                // Validate message
                if !self.validate_message(&message).await? {
                    continue;
                }
                
                // Get consensus from other validators
                let signatures = self.consensus.gather_signatures(&message).await?;
                
                // Submit to bridge contract
                if signatures.len() >= self.consensus.threshold() {
                    self.submit_to_bridge(message, signatures).await?;
                }
            }
            
            tokio::time::sleep(Duration::from_secs(10)).await;
        }
    }
}

// src/validator/message_processor.rs
pub struct MessageProcessor {
    db: Connection,
    monitored_tables: Vec<String>,
}

impl MessageProcessor {
    pub async fn check_tables(&self) -> Result<Vec<Message>> {
        let mut messages = Vec::new();
        
        for table in &self.monitored_tables {
            let sql = format!(
                "SELECT * FROM {} WHERE status = 'pending' ORDER BY id",
                table
            );
            
            let rows = self.db.query(&sql)?;
            
            for row in rows {
                let message = self.parse_message(&table, row)?;
                messages.push(message);
            }
        }
        
        Ok(messages)
    }
    
    fn parse_message(&self, table: &str, row: Row) -> Result<Message> {
        match table {
            "outbound_withdrawals" => {
                Ok(Message::Withdrawal {
                    id: row.get("id")?,
                    account: row.get("account_id")?,
                    token: row.get("token_address")?,
                    amount: row.get("amount")?,
                    destination: row.get("destination_address")?,
                })
            }
            "outbound_messages" => {
                Ok(Message::Generic {
                    id: row.get("id")?,
                    target: row.get("target_contract")?,
                    data: row.get("parameters")?,
                })
            }
            _ => Err(anyhow!("Unknown message table"))
        }
    }
}

// src/validator/settlement.rs
pub struct SettlementPoster {
    contract: BridgeContract,
    signer: Signer,
    batch_size: usize,
}

impl SettlementPoster {
    pub async fn post_state_update(&self, state_update_hash: H256, sequence: u64) -> Result<()> {
        // Generate attestation
        let attestation = self.attestor.generate_attestation(&state_update_hash)?;

        // Get validator signatures
        let signatures = self.consensus.sign_state_update(state_update_hash, sequence).await?;

        // Submit to contract
        let tx = self.contract
            .submit_state_update(state_update_hash, sequence, signatures, attestation)
            .from(self.signer.address())
            .gas_price(self.get_gas_price().await?)
            .send()
            .await?;
            
        info!("State root posted: {:?}", tx.hash());
        Ok(())
    }
}
```

### 5. Extension System

Allow custom validation logic:

```rust
// src/validator/extensions/mod.rs
#[async_trait]
pub trait ValidationExtension: Send + Sync {
    async fn validate_operation(&self, op: &SqlOperation) -> Result<()>;
    async fn validate_message(&self, msg: &Message) -> Result<()>;
    async fn post_replay(&self, state: &DatabaseState) -> Result<()>;
}

// Example: Oracle price verification
pub struct OracleVerifier {
    oracle_client: OracleClient,
    max_deviation: f64,
}

#[async_trait]
impl ValidationExtension for OracleVerifier {
    async fn validate_operation(&self, op: &SqlOperation) -> Result<()> {
        if let Some(price_update) = parse_price_update(op) {
            let oracle_price = self.oracle_client
                .get_price(price_update.token, price_update.timestamp)
                .await?;
                
            let deviation = (price_update.price - oracle_price).abs() / oracle_price;
            
            if deviation > self.max_deviation {
                return Err(anyhow!("Price deviation too high: {}%", deviation * 100.0));
            }
        }
        Ok(())
    }
}

// Example: Custom business rules
pub struct BusinessRuleValidator {
    rules: Vec<Box<dyn Rule>>,
}

pub struct WithdrawalLimitRule {
    daily_limit: u128,
    window: Duration,
}

impl Rule for WithdrawalLimitRule {
    fn validate(&self, op: &SqlOperation) -> Result<()> {
        if let Some(withdrawal) = parse_withdrawal(op) {
            let daily_total = self.get_daily_total(withdrawal.account)?;
            
            if daily_total + withdrawal.amount > self.daily_limit {
                return Err(anyhow!("Daily withdrawal limit exceeded"));
            }
        }
        Ok(())
    }
}
```

## Configuration

### Read Replica Configuration

```yaml
# config/replica.yaml
mode: replica

database:
  path: "/data/replica.db"
  max_connections: 100
  wal_mode: true
  
sync:
  start_sequence: 0  # Start from beginning or specific sequence
  providers:
    celestia:
      enabled: true
      endpoint: "https://rpc.celestia.org"
      namespace: "0x00000000000000000000000000000000synddb"
      priority: 1
    ipfs:
      enabled: true
      endpoint: "http://localhost:5001"
      priority: 2
  batch_size: 100
  retry_attempts: 3
  retry_delay_ms: 1000
  
replay:
  validate_sql: true
  check_invariants: true
  max_transaction_size: 10000
  
api:
  jsonrpc:
    enabled: true
    port: 8545
    max_connections: 1000
    rate_limit: 100  # requests per second
  rest:
    enabled: true
    port: 8080
    cors_origins: ["*"]
  websocket:
    enabled: true
    port: 8546
    max_subscriptions: 10000
    
monitoring:
  prometheus:
    enabled: true
    port: 9090
  tracing:
    enabled: true
    endpoint: "http://localhost:4317"  # OpenTelemetry collector
    
logging:
  level: info
  format: json
```

### Validator Configuration

```yaml
# config/validator.yaml
mode: validator

# Inherits all replica settings plus:
validator:
  enabled: true
  
  # TEE configuration
  tee:
    type: "sgx"  # or "sev", "trustzone"
    mrenclave: "0x..."
    mrsigner: "0x..."
    attestation_endpoint: "https://attestation.service"
    
  # Settlement configuration  
  settlement:
    chain_id: 1
    rpc_endpoint: "https://eth-mainnet.g.alchemy.com/v2/..."
    contract_address: "0x..."
    private_key_path: "/secrets/validator.key"  # Sealed in TEE
    gas_price_multiplier: 1.2
    batch_interval_secs: 300
    min_signatures: 3
    
  # Message processing
  messages:
    monitored_tables:
      - "outbound_withdrawals"
      - "outbound_messages"
      - "outbound_calls"
    process_interval_secs: 10
    max_batch_size: 50
    
  # Consensus with other validators
  consensus:
    validator_endpoints:
      - "https://validator1.synddb.io"
      - "https://validator2.synddb.io"
      - "https://validator3.synddb.io"
    signature_threshold: 2  # 2 of 3
    timeout_secs: 30
    
  # Validation extensions
  extensions:
    oracle_verification:
      enabled: true
      oracle_endpoint: "https://oracle.chainlink.com"
      max_price_deviation: 0.01  # 1%
    custom_rules:
      enabled: true
      rules_path: "/config/custom_rules.yaml"
    rate_limits:
      enabled: true
      withdrawal_daily_limit: "1000000000000000000000"  # 1000 ETH
      message_hourly_limit: 100
```

## Validator TEE Integration with GCP Confidential Space

Validators run in GCP Confidential Space to ensure secure key management and provide attestation for their signing operations. The hardware-protected environment guarantees that validator keys are generated securely and never leave the container.

### Architecture

```
┌─────────────────────────────────────────────────────────────┐
│               GCP Confidential Space Validator              │
│  ┌──────────────────────────────────────────────────────┐  │
│  │           synddb-replica (Validator Mode)             │  │
│  │  ┌────────────────────────────────────────────────┐  │  │
│  │  │  Validator Key Management                       │  │  │
│  │  │  - Generate validator keypair on init          │  │  │
│  │  │  - Store in Secret Manager with WI binding     │  │  │
│  │  │  - Keys bound to container measurements        │  │  │
│  │  └────────────────────────────────────────────────┘  │  │
│  │  ┌────────────────────────────────────────────────┐  │  │
│  │  │  Attestation & Registration                     │  │  │
│  │  │  - Generate attestation token                  │  │  │
│  │  │  - Submit to Bridge.sol with zkProof          │  │  │
│  │  │  - Register public key after verification      │  │  │
│  │  └────────────────────────────────────────────────┘  │  │
│  │  ┌────────────────────────────────────────────────┐  │  │
│  │  │  Message Signing                                │  │  │
│  │  │  - Sign withdrawal messages                    │  │  │
│  │  │  - Sign state updates                         │  │  │
│  │  │  - Include attestation proofs                  │  │  │
│  │  └────────────────────────────────────────────────┘  │  │
│  └──────────────────────────────────────────────────────┘  │
│  Hardware Root of Trust (AMD SEV-SNP / Intel TDX)          │
└─────────────────────────────────────────────────────────────┘
```

### Validator Key Management

```rust
// src/validator/confidential_validator.rs
use gcp_auth::AuthenticationManager;
use google_cloud_secretmanager::client::{Client as SecretClient, ClientConfig};
use google_cloud_default::WithAuthExt;
use k256::{ecdsa::{SigningKey as K256SigningKey, VerifyingKey as K256VerifyingKey, Signature}, SecretKey};
use alloy::signers::Signer;
use sp1_sdk::{ProverClient, SP1Stdin, SP1Proof};
use anyhow::Result;
use serde::{Serialize, Deserialize};

pub struct ConfidentialValidator {
    signing_key: K256SigningKey,
    public_key: K256VerifyingKey,
    ethereum_address: Address,
    secret_client: SecretClient,
    bridge_contract: BridgeContract,
    sp1_client: ProverClient,
    attestation_cache: Arc<RwLock<Option<ValidatorAttestation>>>,
}

#[derive(Serialize, Deserialize)]
struct ValidatorKeyData {
    private_key: Vec<u8>,
    public_key: Vec<u8>,
    ethereum_address: String,
    created_at: i64,
    initial_attestation: String,
    registered_tx_hash: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ValidatorAttestation {
    pub token: String,
    pub public_key: Vec<u8>,
    pub ethereum_address: Address,
    pub container_digest: String,
    pub measured_boot: String,
    pub timestamp: i64,
}

impl ConfidentialValidator {
    pub async fn init(bridge_contract_address: Address, rpc_url: &str) -> Result<Self> {
        let project_id = Self::get_project_id().await?;

        // Initialize Secret Manager client
        let config = ClientConfig::default().with_auth().await?;
        let secret_client = SecretClient::new(config).await?;

        // Validator-specific secret name
        let validator_id = Self::get_instance_id().await?;
        let secret_name = format!("synddb-validator-{}", validator_id);

        // Load or generate validator key
        let (signing_key, public_key, ethereum_address) =
            match Self::load_validator_key(&secret_client, &project_id, &secret_name).await {
                Ok(key_data) => {
                    info!("Loaded existing validator key");
                    let secret_key = SecretKey::from_slice(&key_data.private_key)?;
                    let signing_key = K256SigningKey::from(secret_key);
                    let public_key = signing_key.verifying_key();
                    let address = Address::from_slice(&key_data.ethereum_address);
                    (signing_key, public_key, address)
                }
                Err(_) => {
                    info!("Generating new validator key");
                    Self::generate_and_register_validator_key(
                        &secret_client,
                        &project_id,
                        &secret_name,
                        bridge_contract_address,
                        rpc_url
                    ).await?
                }
            };

        // Initialize SP1 client for zkVM proofs
        let sp1_client = ProverClient::new();

        // Connect to bridge contract
        let provider = Provider::new(Url::parse(rpc_url)?);
        let bridge_contract = BridgeContract::new(bridge_contract_address, provider);

        Ok(Self {
            signing_key,
            public_key,
            ethereum_address,
            secret_client,
            bridge_contract,
            sp1_client,
            attestation_cache: Arc::new(RwLock::new(None)),
        })
    }

    async fn generate_and_register_validator_key(
        secret_client: &SecretClient,
        project_id: &str,
        secret_name: &str,
        bridge_address: Address,
        rpc_url: &str,
    ) -> Result<(K256SigningKey, K256VerifyingKey, Address)> {
        // Generate new key
        let signing_key = K256SigningKey::random(&mut rand::thread_rng());
        let public_key = signing_key.verifying_key();
        let ethereum_address = public_key_to_address(&public_key);

        // Get attestation token
        let attestation = Self::generate_attestation(&public_key).await?;

        // Generate zkVM proof for attestation
        let zk_proof = Self::generate_attestation_proof(&attestation).await?;

        // Register with Bridge.sol
        let provider = Provider::new(Url::parse(rpc_url)?);
        let bridge = BridgeContract::new(bridge_address, provider);

        let tx = bridge
            .registerValidator(
                attestation.token.clone(),
                public_key.to_encoded_point(false).as_bytes().to_vec(),
                zk_proof,
            )
            .send()
            .await?;

        info!("Validator registered on-chain: {:?}", tx.tx_hash());

        // Seal key to Secret Manager
        let key_data = ValidatorKeyData {
            private_key: signing_key.to_bytes().to_vec(),
            public_key: public_key.to_encoded_point(false).as_bytes().to_vec(),
            ethereum_address: format!("{:?}", ethereum_address),
            created_at: chrono::Utc::now().timestamp(),
            initial_attestation: attestation.token,
            registered_tx_hash: Some(format!("{:?}", tx.tx_hash())),
        };

        secret_client
            .create_secret(
                project_id,
                secret_name,
                serde_json::to_vec(&key_data)?,
                Some(vec![
                    ("synddb/role", "validator"),
                    ("synddb/validator-id", &Self::get_instance_id().await?),
                ]),
            )
            .await?;

        Ok((signing_key, public_key, ethereum_address))
    }

    async fn generate_attestation(public_key: &K256VerifyingKey) -> Result<ValidatorAttestation> {
        // Get attestation token from metadata service
        let client = reqwest::Client::new();
        let audience = "https://synddb.io/validator";

        let response = client
            .get("http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token")
            .query(&[
                ("audience", audience),
                ("format", "full"),
                ("licenses", "TRUE"),
            ])
            .header("Metadata-Flavor", "Google")
            .send()
            .await?;

        #[derive(Deserialize)]
        struct TokenResponse {
            token: String,
        }

        let token_resp: TokenResponse = response.json().await?;

        // Parse token to extract measurements
        let token_parts: Vec<&str> = token_resp.token.split('.').collect();
        let payload = base64::decode_config(token_parts[1], base64::URL_SAFE_NO_PAD)?;
        let claims: serde_json::Value = serde_json::from_slice(&payload)?;

        Ok(ValidatorAttestation {
            token: token_resp.token,
            public_key: public_key.to_encoded_point(false).as_bytes().to_vec(),
            ethereum_address: public_key_to_address(public_key),
            container_digest: claims["image_digest"].as_str().unwrap_or("").to_string(),
            measured_boot: claims["measured_boot"].as_str().unwrap_or("").to_string(),
            timestamp: chrono::Utc::now().timestamp(),
        })
    }

    async fn generate_attestation_proof(attestation: &ValidatorAttestation) -> Result<Vec<u8>> {
        // Use SP1 zkVM to generate proof of valid attestation
        let mut stdin = SP1Stdin::new();
        stdin.write(&attestation.token);
        stdin.write(&attestation.public_key);

        // Attestation verification program (pre-compiled)
        let elf = include_bytes!("../../programs/attestation-verifier/elf");

        // Generate proof
        let proof = self.sp1_client.prove(elf, stdin).await?;

        // Serialize proof for on-chain verification
        Ok(bincode::serialize(&proof)?)
    }

    pub async fn sign_message(&self, message: &Message) -> Result<ValidatorSignature> {
        // Hash the message
        let message_hash = keccak256(&abi::encode(&[
            message.id.to_token(),
            message.message_type.to_token(),
            message.schema_hash.to_token(),
            keccak256(&message.payload).to_token(),
            message.nonce.to_token(),
            message.timestamp.to_token(),
        ]));

        // Sign with Ethereum prefix
        let signature = self.signing_key.sign_message(&message_hash)?;

        // Refresh attestation if needed
        let attestation = self.refresh_attestation_if_needed().await?;

        Ok(ValidatorSignature {
            signature: signature.as_bytes().to_vec(),
            signer_address: self.ethereum_address,
            attestation_token: attestation.token,
            timestamp: chrono::Utc::now().timestamp(),
        })
    }

    async fn refresh_attestation_if_needed(&self) -> Result<ValidatorAttestation> {
        let mut cache = self.attestation_cache.write().await;

        let needs_refresh = match &*cache {
            None => true,
            Some(att) => {
                // Refresh every hour
                chrono::Utc::now().timestamp() - att.timestamp > 3600
            }
        };

        if needs_refresh {
            let new_attestation = Self::generate_attestation(&self.public_key).await?;
            *cache = Some(new_attestation.clone());
            Ok(new_attestation)
        } else {
            Ok(cache.as_ref().unwrap().clone())
        }
    }

    pub async fn sign_state_update(&self, state_update_hash: H256, sequence: u64) -> Result<StateUpdateSignature> {
        // Create state update message
        let message = StateUpdateMessage {
            state_update_hash,
            sequence,
            timestamp: chrono::Utc::now().timestamp(),
            validator: self.ethereum_address,
        };

        // Sign the message
        let message_bytes = bincode::serialize(&message)?;
        let signature = self.signing_key.sign_message(&message_bytes)?;

        // Get current attestation
        let attestation = self.refresh_attestation_if_needed().await?;

        Ok(StateUpdateSignature {
            state_update_hash,
            sequence,
            signature: signature.as_bytes().to_vec(),
            validator: self.ethereum_address,
            attestation_token: attestation.token,
        })
    }
}

#[derive(Serialize, Deserialize)]
pub struct ValidatorSignature {
    pub signature: Vec<u8>,
    pub signer_address: Address,
    pub attestation_token: String,
    pub timestamp: i64,
}

#[derive(Serialize, Deserialize)]
pub struct StateUpdateMessage {
    pub state_update_hash: H256,
    pub sequence: u64,
    pub timestamp: i64,
    pub validator: Address,
}

#[derive(Serialize, Deserialize)]
pub struct StateUpdateSignature {
    pub state_update_hash: H256,
    pub sequence: u64,
    pub signature: Vec<u8>,
    pub validator: Address,
    pub attestation_token: String,
}

fn public_key_to_address(public_key: &K256VerifyingKey) -> Address {
    let public_key_bytes = public_key.to_encoded_point(false);
    let hash = keccak256(&public_key_bytes.as_bytes()[1..]); // Skip the 0x04 prefix
    Address::from_slice(&hash[12..])
}
```

### Docker Configuration for Validators

```dockerfile
# Dockerfile.validator-confidential
FROM rust:1.75 as builder

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY programs ./programs

# Build with validator and TEE features
RUN cargo build --release --features "validator,confidential-space"

# Build SP1 attestation verifier program
RUN cd programs/attestation-verifier && \
    cargo prove build

# Runtime image
FROM gcr.io/confidential-space-images/base:latest

RUN apt-get update && apt-get install -y \
    ca-certificates \
    curl \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/synddb-replica /usr/local/bin/
COPY --from=builder /app/programs/attestation-verifier/elf /usr/local/share/synddb/

# Non-root user
RUN useradd -m -u 1000 validator && \
    chown -R validator:validator /usr/local/bin/synddb-replica

USER validator

HEALTHCHECK --interval=30s --timeout=3s \
    CMD curl -f http://localhost:8080/health || exit 1

ENTRYPOINT ["/usr/local/bin/synddb-replica"]
CMD ["--mode", "validator", "--tee", "confidential-space", "--config", "/config/validator.yaml"]
```

### Deployment Configuration

```yaml
# validator-deployment.yaml
apiVersion: apps/v1
kind: StatefulSet
metadata:
  name: synddb-validators
  namespace: synddb
spec:
  serviceName: synddb-validators
  replicas: 3
  selector:
    matchLabels:
      app: synddb-validator
  template:
    metadata:
      labels:
        app: synddb-validator
    spec:
      nodeSelector:
        cloud.google.com/gke-confidential-nodes: "true"

      serviceAccountName: synddb-validator

      containers:
      - name: validator
        image: gcr.io/${PROJECT_ID}/synddb-validator:latest

        env:
        - name: PROJECT_ID
          value: "${PROJECT_ID}"
        - name: VALIDATOR_ID
          valueFrom:
            fieldRef:
              fieldPath: metadata.name
        - name: BRIDGE_CONTRACT
          value: "0x..."
        - name: RPC_URL
          valueFrom:
            secretKeyRef:
              name: synddb-config
              key: rpc-url
        - name: ATTESTATION_AUDIENCE
          value: "https://synddb.io/validator"

        ports:
        - containerPort: 8545  # JSON-RPC
        - containerPort: 8080  # REST
        - containerPort: 9090  # Metrics

        volumeMounts:
        - name: data
          mountPath: /data
        - name: config
          mountPath: /config

        resources:
          requests:
            memory: "8Gi"
            cpu: "4"
          limits:
            memory: "16Gi"
            cpu: "8"

        securityContext:
          runAsNonRoot: true
          runAsUser: 1000
          capabilities:
            drop:
            - ALL

  volumeClaimTemplates:
  - metadata:
      name: data
    spec:
      accessModes: ["ReadWriteOnce"]
      resources:
        requests:
          storage: 500Gi
```

### Configuration

```yaml
# config/validator-confidential.yaml
mode: validator

# Standard replica configuration
database:
  path: "/data/validator.db"
  max_connections: 100

sync:
  providers:
    celestia:
      enabled: true
      endpoint: "https://rpc.celestia.org"

# Validator-specific configuration
validator:
  enabled: true

  # Confidential Space TEE settings
  tee:
    provider: "gcp-confidential-space"

    gcp:
      project_id: "${PROJECT_ID}"
      validator_secret_prefix: "synddb-validator"
      attestation_audience: "https://synddb.io/validator"

      # Workload Identity configuration
      service_account: "synddb-validator@${PROJECT_ID}.iam.gserviceaccount.com"

      # Expected measurements
      expected_measurements:
        container_digest: "${EXPECTED_VALIDATOR_IMAGE_DIGEST}"

    # Attestation refresh
    attestation_refresh_mins: 60

  # Bridge contract interaction
  settlement:
    chain_id: 1
    rpc_endpoint: "${RPC_URL}"
    contract_address: "${BRIDGE_CONTRACT}"
    gas_price_multiplier: 1.2

  # Message processing
  messages:
    monitored_tables:
      - "outbound_withdrawals"
      - "outbound_messages"
    process_interval_secs: 10
    batch_size: 50

  # Coordination with other validators
  consensus:
    # Validators discover each other via k8s service
    service_name: "synddb-validators"
    namespace: "synddb"
    port: 8545

    # Minimum signatures required
    signature_threshold: 2

    # Timeout for gathering signatures
    timeout_secs: 30

  # zkVM proof generation
  zk_proof:
    enabled: true
    program_path: "/usr/local/share/synddb/attestation-verifier.elf"
    max_proof_generation_time_secs: 60

monitoring:
  metrics:
    enabled: true
    port: 9090

  health:
    enabled: true
    port: 8080
    checks:
      - attestation_validity
      - key_accessibility
      - bridge_connectivity
```

## Testing Strategy

### Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_sql_replay() {
        let replayer = SqlReplayer::new_test();
        let batch = SqlBatch {
            operations: vec![
                SqlOperation::new("INSERT INTO users VALUES (1, 'alice')"),
                SqlOperation::new("UPDATE users SET name = 'bob' WHERE id = 1"),
            ],
            ..Default::default()
        };
        
        replayer.replay_batch(batch).await.unwrap();
        
        let result = replayer.query("SELECT name FROM users WHERE id = 1").await.unwrap();
        assert_eq!(result[0]["name"], "bob");
    }
    
    #[test]
    fn test_invariant_checker() {
        let checker = BalanceInvariant::new();
        let mut conn = Connection::open_in_memory().unwrap();
        
        // Setup test data
        conn.execute("CREATE TABLE balances (account TEXT, amount INTEGER)", []).unwrap();
        conn.execute("INSERT INTO balances VALUES ('alice', -100)", []).unwrap();
        
        // Should fail on negative balance
        assert!(checker.check(&conn).is_err());
    }
}
```

### Integration Tests

```rust
#[tokio::test]
async fn test_full_sync() {
    // Start mock DA provider
    let mock_da = MockDaProvider::new();
    mock_da.add_packet(create_test_packet());
    
    // Start replica
    let config = Config::test();
    let replica = Replica::new(config);
    replica.start().await;
    
    // Wait for sync
    tokio::time::sleep(Duration::from_secs(2)).await;
    
    // Query synced data
    let client = JsonRpcClient::new("http://localhost:8545");
    let result = client.query("SELECT COUNT(*) FROM test").await.unwrap();
    assert_eq!(result, 1);
}
```

### Benchmarks

```rust
use criterion::{criterion_group, criterion_main, Criterion};

fn bench_sql_replay(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    
    c.bench_function("replay_1000_ops", |b| {
        b.iter(|| {
            rt.block_on(async {
                let replayer = SqlReplayer::new_test();
                let batch = generate_batch(1000);
                replayer.replay_batch(batch).await.unwrap();
            })
        })
    });
}
```

## Deployment

### Docker Image

```dockerfile
# Builder stage
FROM rust:1.75 as builder
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release --features tee

# Runtime stage
FROM ubuntu:22.04
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libsgx-dcap-ql \
    libsgx-urts
    
COPY --from=builder /app/target/release/synddb-replica /usr/local/bin/
COPY config /etc/synddb/

ENTRYPOINT ["synddb-replica"]
CMD ["--config", "/etc/synddb/config.yaml"]
```

### Kubernetes Deployment

```yaml
apiVersion: apps/v1
kind: StatefulSet
metadata:
  name: synddb-replica
spec:
  serviceName: synddb-replica
  replicas: 3
  template:
    spec:
      containers:
      - name: replica
        image: syndicate/synddb-replica:latest
        ports:
        - containerPort: 8545  # JSON-RPC
        - containerPort: 8080  # REST
        - containerPort: 8546  # WebSocket
        volumeMounts:
        - name: data
          mountPath: /data
        resources:
          requests:
            memory: "4Gi"
            cpu: "2"
          limits:
            memory: "8Gi"
            cpu: "4"
  volumeClaimTemplates:
  - metadata:
      name: data
    spec:
      accessModes: ["ReadWriteOnce"]
      resources:
        requests:
          storage: 100Gi
```

## Performance Optimizations

### 1. Parallel DA Fetching
```rust
let futures = providers.iter().map(|p| p.fetch_latest());
let results = futures::future::join_all(futures).await;
```

### 2. Connection Pooling
```rust
let pool = SqlitePool::new()
    .max_connections(100)
    .min_connections(10)
    .connection_timeout(Duration::from_secs(5))
    .build()?;
```

### 3. Prepared Statement Caching
```rust
let mut stmt_cache = LruCache::new(100);
if let Some(stmt) = stmt_cache.get(sql) {
    stmt.execute(params)?;
} else {
    let stmt = conn.prepare(sql)?;
    stmt_cache.put(sql.to_string(), stmt);
}
```

### 4. Read Replicas Load Balancing
```rust
let replicas = vec![replica1, replica2, replica3];
let selected = replicas[rand::random::<usize>() % replicas.len()];
selected.query(sql).await
```

## Security Considerations

### 1. SQL Injection Prevention
```rust
// Validate all SQL before execution
let parsed = sqlite_parser::parse(sql)?;
if contains_dangerous_operations(&parsed) {
    return Err(SecurityError::DangerousSql);
}
```

### 2. Rate Limiting
```rust
use governor::{Quota, RateLimiter};

let limiter = RateLimiter::direct(Quota::per_second(100));
if limiter.check().is_err() {
    return Err(Error::RateLimited);
}
```

### 3. Access Control
```rust
pub struct AccessControl {
    allowed_tables: HashSet<String>,
    read_only: bool,
}

impl AccessControl {
    pub fn check(&self, sql: &str) -> Result<()> {
        if self.read_only && !is_select_query(sql) {
            return Err(Error::ReadOnlyMode);
        }
        // Check table access
        for table in extract_tables(sql)? {
            if !self.allowed_tables.contains(&table) {
                return Err(Error::TableNotAllowed(table));
            }
        }
        Ok(())
    }
}
```

## Resource Requirements

### Read Replica
- **CPU**: 4 cores recommended
- **Memory**: 4GB minimum, 8GB recommended
- **Disk**: 100GB SSD minimum
- **Network**: 100Mbps minimum

### Validator
- **CPU**: 8 cores recommended (TEE-enabled)
- **Memory**: 16GB minimum
- **Disk**: 500GB SSD
- **Network**: 1Gbps recommended
- **TEE**: Intel SGX, AMD SEV, or ARM TrustZone

## Monitoring Metrics

Key metrics exposed via Prometheus:
- `synddb_sync_lag_seconds` - How far behind the replica is
- `synddb_sql_operations_replayed` - Total operations replayed
- `synddb_validation_failures` - Failed validations
- `synddb_query_latency_ms` - Query response time
- `synddb_state_update_submissions` - Successful settlements (validator only)
- `synddb_message_processing_time` - Message processing latency (validator only)
