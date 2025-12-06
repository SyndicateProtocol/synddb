# PLAN_VALIDATOR.md - SyndDB Validator Implementation

## Overview

The `synddb-validator` syncs state from DA layers, validates all sequenced messages, and serves queries. All validators perform the same core validation - the `--bridge-signer` flag enables additional functionality for signing withdrawal approvals and state attestations for the bridge contract.

**Modes:**
- **Default**: Sync, validate, serve queries (read-only replica functionality)
- **`--bridge-signer`**: Additionally sign for bridge contract (withdrawals, state roots)

**Key Integration Points:**
- Consumes `SignedMessage` from sequencer's DA publishers (GCS, Celestia, etc.)
- Applies SQLite changesets (binary format from Session Extension), not SQL statements
- Verifies sequencer signatures using secp256k1 (same scheme as sequencer)
- Reuses `synddb-chain-monitor` for blockchain event handling
- Bridge signers produce signatures that relayers submit to the bridge contract

## Architecture

```
┌──────────────────────────────────────────────────────────────┐
│                        DA Layers                             │
│  ┌──────────┐ ┌──────────┐ ┌──────┐ ┌──────────┐           │
│  │   GCS    │ │ Celestia │ │ IPFS │ │ EigenDA  │           │
│  └──────────┘ └──────────┘ └──────┘ └──────────┘           │
└──────────────────────────────────────────────────────────────┘
                            ↓
┌──────────────────────────────────────────────────────────────┐
│                   synddb-validator                           │
│ ┌──────────────────────────────────────────────────────────┐│
│ │                    DA Syncer                             ││
│ │  ┌────────────┐  ┌────────────┐  ┌────────────┐         ││
│ │  │ Fetcher    │→ │ Verifier   │→ │  Orderer   │         ││
│ │  │(GCS/DA)    │  │(Signature) │  │ (Sequence) │         ││
│ │  └────────────┘  └────────────┘  └────────────┘         ││
│ └──────────────────────────────────────────────────────────┘│
│                           ↓                                  │
│ ┌──────────────────────────────────────────────────────────┐│
│ │               Changeset Applier                          ││
│ │  ┌────────────┐  ┌────────────┐  ┌────────────┐         ││
│ │  │Decompress  │→ │  Apply     │→ │ Validate   │         ││
│ │  │  (zstd)    │  │ (Session)  │  │(Invariants)│         ││
│ │  └────────────┘  └────────────┘  └────────────┘         ││
│ └──────────────────────────────────────────────────────────┘│
│                           ↓                                  │
│              ┌──────────────────────────┐                   │
│              │     Local SQLite DB      │                   │
│              └──────────────────────────┘                   │
│                    ↓              ↓                          │
│ ┌─────────────────────┐  ┌──────────────────────────────┐  │
│ │   Query Server      │  │  Bridge Signer (optional)    │  │
│ │  ┌──────────────┐   │  │  ┌────────────────────────┐  │  │
│ │  │  JSON-RPC    │   │  │  │ Withdrawal Signer      │  │  │
│ │  └──────────────┘   │  │  └────────────────────────┘  │  │
│ │  ┌──────────────┐   │  │  ┌────────────────────────┐  │  │
│ │  │  REST API    │   │  │  │ State Attestor         │  │  │
│ │  └──────────────┘   │  │  └────────────────────────┘  │  │
│ │  ┌──────────────┐   │  │  ┌────────────────────────┐  │  │
│ │  │  WebSocket   │   │  │  │ TEE Attestation        │  │  │
│ │  └──────────────┘   │  │  └────────────────────────┘  │  │
│ └─────────────────────┘  └──────────────────────────────┘  │
└──────────────────────────────────────────────────────────────┘
                                  ↓ (signatures only)
                    ┌─────────────────────────┐
                    │   Relayer (separate)    │
                    │   Submits to Bridge     │
                    └─────────────────────────┘
```

## Data Formats (Aligned with Sequencer)

### SignedMessage (from sequencer)

The replica fetches `SignedMessage` objects from DA layers. This is the exact format produced by `synddb-sequencer`.

See: [`crates/synddb-shared/src/types/message.rs`](crates/synddb-shared/src/types/message.rs)

### Payload Formats (after zstd decompression)

All payload types are defined in [`crates/synddb-shared/src/types/payloads.rs`](crates/synddb-shared/src/types/payloads.rs):

- **Changeset Batch** (MessageType::Changeset): `ChangesetBatchRequest` containing `Vec<ChangesetData>`
- **Snapshot** (MessageType::Snapshot): `SnapshotRequest` containing `SnapshotData`
- **Withdrawal** (MessageType::Withdrawal): `WithdrawalRequest`

## Core Libraries

See [`crates/synddb-validator/Cargo.toml`](crates/synddb-validator/Cargo.toml) for current dependencies.

## Directory Structure

```
synddb-validator/
├── Cargo.toml
├── src/
│   ├── main.rs                      # Entry point
│   ├── lib.rs                       # Public API
│   ├── config.rs                    # Configuration (clap + env vars)
│   ├── sync/
│   │   ├── mod.rs                   # DA syncing orchestration
│   │   ├── fetcher.rs               # Fetch SignedMessage from DA
│   │   ├── verifier.rs              # Verify sequencer signatures
│   │   ├── state_manager.rs         # Track sync state (SQLite)
│   │   └── providers/
│   │       ├── mod.rs               # DAFetcher trait
│   │       ├── gcs.rs               # GCS fetcher (primary)
│   │       ├── celestia.rs          # Celestia fetcher
│   │       └── mock.rs              # Mock for testing
│   ├── apply/
│   │   ├── mod.rs                   # Changeset application engine
│   │   ├── applier.rs               # Apply SQLite changesets
│   │   ├── snapshot.rs              # Restore from snapshots
│   │   ├── invariants.rs            # Post-apply invariant checks
│   │   └── types.rs                 # Shared types
│   ├── database/
│   │   ├── mod.rs                   # SQLite management
│   │   ├── pool.rs                  # Read connection pool
│   │   └── state.rs                 # Validator state tracking
│   ├── api/
│   │   ├── mod.rs                   # API servers
│   │   ├── rest.rs                  # REST API (axum)
│   │   ├── jsonrpc.rs               # JSON-RPC server
│   │   └── websocket.rs             # WebSocket subscriptions
│   ├── bridge/                      # Bridge signer functionality (--bridge-signer)
│   │   ├── mod.rs                   # Bridge signer orchestration
│   │   ├── withdrawal_signer.rs     # Sign withdrawal approvals
│   │   ├── state_attestor.rs        # Sign state root attestations
│   │   └── signature_store.rs       # Store/serve signatures for relayers
│   ├── tee/
│   │   ├── mod.rs                   # TEE integration
│   │   ├── confidential_space.rs    # GCP Confidential Space
│   │   └── attestation.rs           # Generate/verify attestations
│   └── metrics.rs                   # Prometheus metrics
├── tests/
│   ├── integration/
│   │   ├── sync_test.rs             # End-to-end sync tests
│   │   └── apply_test.rs            # Changeset application tests
│   └── fixtures/                    # Test data
└── README.md
```

## Core Components

### 1. DA Syncer

Fetches `SignedMessage` from DA layers and verifies sequencer signatures. The trait mirrors the sequencer's `DAPublisher` interface for consistency.

See:
- [`crates/synddb-validator/src/sync/mod.rs`](crates/synddb-validator/src/sync/mod.rs) - DAFetcher trait and sync logic
- [`crates/synddb-validator/src/sync/providers/gcs.rs`](crates/synddb-validator/src/sync/providers/gcs.rs) - GCS fetcher implementation

### 2. Signature Verifier

Verifies sequencer signatures using the same scheme as `synddb-sequencer`.

See: [`crates/synddb-validator/src/sync/verifier.rs`](crates/synddb-validator/src/sync/verifier.rs)

### 3. Changeset Applier

Applies SQLite changesets from the sequencer using rusqlite's Session Extension.

See: [`crates/synddb-validator/src/apply/mod.rs`](crates/synddb-validator/src/apply/mod.rs)

### 4. Query Server

Serves queries via REST API. See [`crates/synddb-validator/src/http.rs`](crates/synddb-validator/src/http.rs) for current implementation.

Future protocols: JSON-RPC, WebSocket subscriptions.

### 5. Bridge Signer Mode

When `--bridge-signer` is enabled, the validator signs withdrawal approvals and state attestations for the bridge contract.

**Submission modes:**
- **Relayer mode** (default, `--bridge-submit=false`): Validator signs and stores signatures. A separate relayer collects signatures from multiple validators via the signature API and submits to the bridge.
- **Direct mode** (`--bridge-submit=true`): Validator signs and submits transactions directly to the bridge contract. Useful for single-validator setups or when running your own relayer.

**Bridge contract interactions:**
- Read validator registration status
- Read signature threshold requirements
- Submit withdrawals (direct mode only)
- Submit state attestations (direct mode only)

See: [`crates/synddb-validator/src/bridge/mod.rs`](crates/synddb-validator/src/bridge/mod.rs)

### Signature API Reference

The signature endpoint (default `:8081`) serves signatures for relayers:

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/signatures/withdrawal/:request_id` | GET | Get all signatures for a withdrawal |
| `/signatures/state/:sequence` | GET | Get state attestations for a sequence |
| `/signatures/pending` | GET | List withdrawal IDs with pending signatures |
| `/health` | GET | Health check |

**Example: Fetch withdrawal signatures**
```bash
curl http://validator:8081/signatures/withdrawal/0x1234...

# Response
[
  {
    "request_id": "0x1234...",
    "recipient": "0xabcd...",
    "amount": "1000000000000000000",
    "sequence": 42,
    "signature": "0x...",
    "signer": "0x9876..."
  }
]
```

**Relayer workflow:**
1. Poll `/signatures/pending` for new withdrawal IDs
2. For each ID, fetch signatures from multiple validators
3. Once threshold signatures collected, submit to bridge contract
4. Bridge contract verifies signatures and processes withdrawal

### 6. Extension System

Allow custom validation logic (simplified from original - focuses on withdrawal validation):

```rust
// src/validator/extensions.rs

#[async_trait]
pub trait WithdrawalValidator: Send + Sync {
    /// Validate a withdrawal before it's posted to L1
    async fn validate(&self, withdrawal: &PendingWithdrawal) -> Result<()>;
}

/// Rate limit withdrawals per address
pub struct WithdrawalRateLimiter {
    daily_limit: U256,
    limits: Arc<DashMap<Address, DailyLimit>>,
}

impl WithdrawalValidator for WithdrawalRateLimiter {
    async fn validate(&self, withdrawal: &PendingWithdrawal) -> Result<()> {
        let today = chrono::Utc::now().date_naive();

        let mut entry = self.limits
            .entry(withdrawal.recipient)
            .or_insert_with(|| DailyLimit {
                date: today,
                total: U256::ZERO,
            });

        // Reset if new day
        if entry.date != today {
            entry.date = today;
            entry.total = U256::ZERO;
        }

        if entry.total + withdrawal.amount > self.daily_limit {
            return Err(anyhow!(
                "Daily withdrawal limit exceeded for {}",
                withdrawal.recipient
            ));
        }

        entry.total += withdrawal.amount;
        Ok(())
    }
}
```

## Configuration

Configuration follows the project pattern: clap derive with env var support, serde for serialization, and `humantime-serde` for durations.

### Validator Configuration (src/config.rs)

The configuration below reflects the **current implementation**:

```rust
use clap::Parser;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::time::Duration;

/// SyndDB Validator - syncs, validates, and optionally signs for bridge
#[derive(Debug, Clone, Serialize, Deserialize, Parser)]
#[command(name = "synddb-validator")]
#[command(about = "SyndDB Validator - validates sequencer messages and applies changesets")]
pub struct ValidatorConfig {
    // === Core Validation (always required) ===

    /// Path to the SQLite database file for replicated state
    #[arg(long, env = "DATABASE_PATH", default_value = "/data/validator.db")]
    pub database_path: String,

    /// Path to the SQLite database file for validator state (sequences, etc.)
    #[arg(long, env = "STATE_DB_PATH", default_value = "/data/validator_state.db")]
    pub state_db_path: String,

    /// Expected sequencer address (for signature verification)
    #[arg(long, env = "SEQUENCER_ADDRESS")]
    pub sequencer_address: String,

    /// GCS bucket for fetching messages
    #[arg(long, env = "GCS_BUCKET")]
    pub gcs_bucket: Option<String>,

    /// GCS path prefix (must match sequencer)
    #[arg(long, env = "GCS_PREFIX", default_value = "sequencer")]
    pub gcs_prefix: String,

    // === HTTP Server ===

    /// HTTP API bind address
    #[arg(long, env = "BIND_ADDRESS", default_value = "0.0.0.0:8080")]
    pub bind_address: SocketAddr,

    // === Timing ===

    /// Sync poll interval
    #[arg(long, env = "SYNC_INTERVAL", default_value = "1s", value_parser = humantime::parse_duration)]
    #[serde(with = "humantime_serde")]
    pub sync_interval: Duration,

    /// Sequence number to start syncing from (0 means start from beginning)
    #[arg(long, env = "START_SEQUENCE", default_value = "0")]
    pub start_sequence: u64,

    /// Graceful shutdown timeout
    #[arg(long, env = "SHUTDOWN_TIMEOUT", default_value = "30s", value_parser = humantime::parse_duration)]
    #[serde(with = "humantime_serde")]
    pub shutdown_timeout: Duration,

    // === Bridge Signer Mode ===

    /// Enable bridge signer mode - signs withdrawal messages for bridge contract
    #[arg(long, env = "BRIDGE_SIGNER")]
    pub bridge_signer: bool,

    /// Bridge contract address (required if --bridge-signer)
    #[arg(long, env = "BRIDGE_CONTRACT")]
    pub bridge_contract: Option<String>,

    /// Chain ID for the bridge contract (required if --bridge-signer)
    #[arg(long, env = "BRIDGE_CHAIN_ID")]
    pub bridge_chain_id: Option<u64>,

    /// Signing key for bridge operations (hex private key, required if --bridge-signer)
    #[arg(long, env = "BRIDGE_SIGNING_KEY")]
    pub bridge_signing_key: Option<String>,

    /// Endpoint to serve signatures for relayers
    #[arg(long, env = "BRIDGE_SIGNATURE_ENDPOINT", default_value = "0.0.0.0:8081")]
    pub bridge_signature_endpoint: SocketAddr,

    // === Gap Detection ===

    /// Maximum number of retries when a sequence gap is detected
    #[arg(long, env = "GAP_RETRY_COUNT", default_value = "5")]
    pub gap_retry_count: u32,

    /// Delay between gap retry attempts
    #[arg(long, env = "GAP_RETRY_DELAY", default_value = "5s", value_parser = humantime::parse_duration)]
    #[serde(with = "humantime_serde")]
    pub gap_retry_delay: Duration,

    /// Skip gaps after max retries instead of erroring (use with caution)
    #[arg(long, env = "GAP_SKIP_ON_FAILURE", default_value = "false")]
    pub gap_skip_on_failure: bool,

    // === Logging ===

    /// Enable JSON log format (for production log aggregation)
    #[arg(long, env = "LOG_JSON", default_value = "false")]
    pub log_json: bool,
}

impl ValidatorConfig {
    /// Create a config for testing with a specific sequencer address
    pub fn with_sequencer_address(address: &str) -> Self {
        Self::parse_from([
            "synddb-validator",
            "--sequencer-address",
            address,
            "--database-path",
            ":memory:",
            "--state-db-path",
            ":memory:",
        ])
    }

    /// Check if bridge signer mode is enabled
    pub const fn is_bridge_signer(&self) -> bool {
        self.bridge_signer
    }

    /// Validate bridge signer configuration
    pub fn validate_bridge_config(&self) -> Result<(), String> {
        if !self.bridge_signer {
            return Ok(());
        }
        if self.bridge_contract.is_none() {
            return Err("--bridge-contract is required when --bridge-signer is enabled".into());
        }
        if self.bridge_chain_id.is_none() {
            return Err("--bridge-chain-id is required when --bridge-signer is enabled".into());
        }
        if self.bridge_signing_key.is_none() {
            return Err("--bridge-signing-key is required when --bridge-signer is enabled".into());
        }
        Ok(())
    }
}
```

### Usage Examples

```bash
# Basic validator - syncs, validates, serves queries
synddb-validator \
  --sequencer-address=0x742d35Cc6634C0532925a3b844Bc9e7595f2bD41 \
  --gcs-bucket=synddb-messages

# Bridge signer mode - signs withdrawal messages for relayers
synddb-validator \
  --sequencer-address=0x742d35Cc6634C0532925a3b844Bc9e7595f2bD41 \
  --gcs-bucket=synddb-messages \
  --bridge-signer \
  --bridge-contract=0x1234567890abcdef1234567890abcdef12345678 \
  --bridge-chain-id=1 \
  --bridge-signing-key=0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80

# With custom gap handling for unreliable DA sources
synddb-validator \
  --sequencer-address=0x742d35Cc6634C0532925a3b844Bc9e7595f2bD41 \
  --gcs-bucket=synddb-messages \
  --gap-retry-count=10 \
  --gap-retry-delay=10s
```

### Environment Variables

```bash
# Required for all validators
export SEQUENCER_ADDRESS="0x..."      # Sequencer's Ethereum address
export GCS_BUCKET="synddb-messages"   # GCS bucket with sequenced messages

# Optional (with defaults)
export DATABASE_PATH="/data/validator.db"
export STATE_DB_PATH="/data/validator_state.db"
export GCS_PREFIX="sequencer"
export START_SEQUENCE="0"
export BIND_ADDRESS="0.0.0.0:8080"
export SYNC_INTERVAL="1s"
export SHUTDOWN_TIMEOUT="30s"
export LOG_JSON="false"

# Gap detection (optional, with defaults)
export GAP_RETRY_COUNT="5"
export GAP_RETRY_DELAY="5s"
export GAP_SKIP_ON_FAILURE="false"

# Bridge signer mode (all required if BRIDGE_SIGNER=true)
export BRIDGE_SIGNER="true"
export BRIDGE_CONTRACT="0x..."
export BRIDGE_CHAIN_ID="1"
export BRIDGE_SIGNING_KEY="0x..."
export BRIDGE_SIGNATURE_ENDPOINT="0.0.0.0:8081"
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
    use rusqlite::Connection;

    #[test]
    fn test_signature_verification() {
        let verifier = SignatureVerifier::new();

        // Create a test message (would need actual signed message from sequencer)
        let message = SignedMessage {
            sequence: 1,
            timestamp: 1700000000,
            message_type: MessageType::Changeset,
            payload: vec![0x01, 0x02, 0x03],
            message_hash: "0x...".to_string(),
            signature: "0x...".to_string(),
            signer: "0x742d35Cc6634C0532925a3b844Bc9e7595f2bD41".to_string(),
        };

        let expected_signer: Address = "0x742d35Cc6634C0532925a3b844Bc9e7595f2bD41".parse().unwrap();

        // Would verify against actual test vectors
        // assert!(verifier.verify(&message, expected_signer).is_ok());
    }

    #[test]
    fn test_changeset_apply() {
        // Create in-memory database with schema
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)", []).unwrap();
        conn.execute("INSERT INTO users VALUES (1, 'alice')", []).unwrap();

        // Create a changeset using Session API
        let mut session = rusqlite::session::Session::new(&conn).unwrap();
        session.attach(None).unwrap();  // Attach to all tables

        // Make a change
        conn.execute("UPDATE users SET name = 'bob' WHERE id = 1", []).unwrap();

        // Get the changeset
        let changeset = session.changeset().unwrap();

        // Now apply it to another database
        let mut target = Connection::open_in_memory().unwrap();
        target.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)", []).unwrap();
        target.execute("INSERT INTO users VALUES (1, 'alice')", []).unwrap();

        // Apply changeset
        let cs = rusqlite::session::Changeset::new(&changeset).unwrap();
        cs.apply(&target, None::<fn(&str) -> bool>, |_| {
            rusqlite::session::ConflictAction::Abort
        }).unwrap();

        // Verify
        let name: String = target.query_row(
            "SELECT name FROM users WHERE id = 1",
            [],
            |row| row.get(0)
        ).unwrap();
        assert_eq!(name, "bob");
    }

    #[test]
    fn test_invariant_checker() {
        let checker = NoNegativeBalances {
            table: "balances".to_string(),
            column: "amount".to_string(),
        };
        let conn = Connection::open_in_memory().unwrap();

        // Setup test data
        conn.execute("CREATE TABLE balances (account TEXT, amount INTEGER)", []).unwrap();
        conn.execute("INSERT INTO balances VALUES ('alice', -100)", []).unwrap();

        // Should fail on negative balance
        assert!(checker.check(&conn).is_err());
    }

    #[test]
    fn test_zstd_decompression() {
        let original = b"test data for compression";

        // Compress
        let compressed = zstd::encode_all(&original[..], 3).unwrap();

        // Decompress
        let decompressed = zstd::decode_all(&compressed[..]).unwrap();

        assert_eq!(&decompressed, original);
    }
}
```

### Integration Tests

```rust
#[tokio::test]
async fn test_full_sync() {
    // Start mock DA fetcher
    let mock_fetcher = Arc::new(MockDAFetcher::new());
    mock_fetcher.add_message(create_test_signed_message(1));
    mock_fetcher.add_message(create_test_signed_message(2));

    // Create replica with in-memory database
    let config = ReplicaConfig::for_testing(":memory:", "0x...");
    let (tx, rx) = tokio::sync::mpsc::channel(100);

    // Start syncer
    let state_manager = StateManager::new(":memory:").unwrap();
    let expected_signer = config.sequencer_address.parse().unwrap();
    let syncer = DaSyncer::new(vec![mock_fetcher], state_manager, expected_signer);

    // Start applier in background
    let mut applier = ChangesetApplier::new(":memory:", None).unwrap();
    let applier_handle = tokio::spawn(async move {
        applier.run(rx).await
    });

    // Run syncer briefly
    tokio::time::timeout(Duration::from_secs(2), syncer.run(tx)).await.ok();

    // Verify messages were applied
    // ...
}

fn create_test_signed_message(sequence: u64) -> SignedMessage {
    // Create a minimal test message
    SignedMessage {
        sequence,
        timestamp: chrono::Utc::now().timestamp() as u64,
        message_type: MessageType::Changeset,
        payload: zstd::encode_all(&b"{\"batch_id\":\"test\",\"changesets\":[]}"[..], 3).unwrap(),
        message_hash: "0x0000000000000000000000000000000000000000000000000000000000000000".to_string(),
        signature: "0x".to_string() + &"00".repeat(65),
        signer: "0x0000000000000000000000000000000000000000".to_string(),
    }
}
```

### Benchmarks

```rust
use criterion::{criterion_group, criterion_main, Criterion};

fn bench_changeset_apply(c: &mut Criterion) {
    c.bench_function("apply_changeset", |b| {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)", []).unwrap();

        // Pre-create a changeset
        let changeset = create_test_changeset();

        b.iter(|| {
            let cs = rusqlite::session::Changeset::new(&changeset).unwrap();
            cs.apply(&conn, None::<fn(&str) -> bool>, |_| {
                rusqlite::session::ConflictAction::Abort
            }).unwrap();
        })
    });
}

fn bench_zstd_decompress(c: &mut Criterion) {
    // Compress 1MB of test data
    let data = vec![0u8; 1024 * 1024];
    let compressed = zstd::encode_all(&data[..], 3).unwrap();

    c.bench_function("zstd_decompress_1mb", |b| {
        b.iter(|| {
            zstd::decode_all(&compressed[..]).unwrap()
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

### 1. Signature Verification
```rust
// All messages must be signed by the expected sequencer
// Signature verification happens before any data is applied
fn verify_message(&self, message: &SignedMessage) -> Result<()> {
    // Verify message_hash matches payload
    let computed_hash = keccak256(&message.payload);
    if computed_hash != message.message_hash {
        return Err(anyhow!("Payload hash mismatch"));
    }

    // Verify signature recovers to expected sequencer
    let recovered = recover_signer(&message)?;
    if recovered != self.expected_sequencer {
        return Err(anyhow!("Invalid sequencer signature"));
    }

    Ok(())
}
```

### 2. Read-Only Query Enforcement
```rust
// Replica serves read-only queries - no writes allowed through API
pub fn validate_query(sql: &str) -> Result<()> {
    let normalized = sql.trim().to_uppercase();
    if !normalized.starts_with("SELECT") {
        return Err(Error::ReadOnlyMode);
    }
    Ok(())
}
```

### 3. Rate Limiting
```rust
use tower::limit::RateLimitLayer;

let rate_limit = RateLimitLayer::new(100, Duration::from_secs(1));
let app = Router::new()
    .route("/query", post(query_handler))
    .layer(rate_limit);
```

### 4. Changeset Validation
```rust
// Changesets are applied atomically with conflict detection
fn apply_changeset(&self, data: &[u8]) -> Result<()> {
    let changeset = rusqlite::session::Changeset::new(data)?;

    // Apply with strict conflict handling - abort on any conflict
    changeset.apply(&self.conn, None::<fn(&str) -> bool>, |conflict| {
        error!("Changeset conflict: {:?}", conflict);
        rusqlite::session::ConflictAction::Abort
    })?;

    Ok(())
}
```

## Resource Requirements

### Read Replica
- **CPU**: 2+ cores
- **Memory**: 2GB minimum, 4GB recommended
- **Disk**: 50GB+ SSD (depends on database size)
- **Network**: 100Mbps minimum

### Validator
- **CPU**: 4+ cores (TEE-enabled for Confidential Space)
- **Memory**: 8GB minimum, 16GB recommended
- **Disk**: 200GB+ SSD
- **Network**: 1Gbps recommended
- **TEE**: GCP Confidential Space (AMD SEV-SNP)

## Monitoring Metrics

Key metrics exposed via Prometheus:

```rust
// src/metrics.rs
use prometheus::{IntCounter, IntGauge, Histogram};

lazy_static! {
    pub static ref SYNC_LAG: IntGauge = IntGauge::new(
        "synddb_sync_lag_sequences",
        "Number of sequences behind the latest"
    ).unwrap();

    pub static ref MESSAGES_APPLIED: IntCounter = IntCounter::new(
        "synddb_messages_applied_total",
        "Total messages applied"
    ).unwrap();

    pub static ref CHANGESETS_APPLIED: IntCounter = IntCounter::new(
        "synddb_changesets_applied_total",
        "Total changesets applied"
    ).unwrap();

    pub static ref SNAPSHOTS_APPLIED: IntCounter = IntCounter::new(
        "synddb_snapshots_applied_total",
        "Total snapshots restored"
    ).unwrap();

    pub static ref SIGNATURE_FAILURES: IntCounter = IntCounter::new(
        "synddb_signature_verification_failures_total",
        "Failed signature verifications"
    ).unwrap();

    pub static ref QUERY_LATENCY: Histogram = Histogram::with_opts(
        HistogramOpts::new("synddb_query_latency_seconds", "Query latency")
    ).unwrap();

    // Validator-only metrics
    pub static ref WITHDRAWALS_PROCESSED: IntCounter = IntCounter::new(
        "synddb_withdrawals_processed_total",
        "Withdrawals posted to L1"
    ).unwrap();
}
```

Key metrics:
- `synddb_sync_lag_sequences` - How many sequences behind the replica is
- `synddb_messages_applied_total` - Total messages processed
- `synddb_changesets_applied_total` - Total changesets applied
- `synddb_snapshots_applied_total` - Total snapshots restored
- `synddb_signature_verification_failures_total` - Failed signature verifications
- `synddb_query_latency_seconds` - Query response time histogram
- `synddb_withdrawals_processed_total` - Withdrawals posted to L1 (validator only)
